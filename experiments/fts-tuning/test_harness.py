"""Tests for the FTS tuning harness. Run: python3 -m unittest experiments/fts-tuning/test_harness.py"""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import hashlib  # noqa: E402
import json  # noqa: E402

import incremental_optimize  # noqa: E402
import index_baseline  # noqa: E402
import optimize  # noqa: E402
import search_baseline  # noqa: E402


class SqliteVersionGuard(unittest.TestCase):
    metadata = {
        "inspection_sqlite": "3.54.1 full baseline version",
        "bundled_sqlite": "3.54.1",
    }

    def test_recorded_matching_cli_is_accepted(self):
        with mock.patch.object(
            optimize.subprocess,
            "check_output",
            return_value="3.54.1 full baseline version\n",
        ):
            optimize.check_sqlite_version(self.metadata)

    def test_changed_cli_is_rejected(self):
        with mock.patch.object(
            optimize.subprocess, "check_output", return_value="3.54.1 another build\n"
        ):
            with self.assertRaisesRegex(ValueError, "differs from the index baseline"):
                optimize.check_sqlite_version(self.metadata)

    def test_cli_must_match_bundled_version(self):
        with mock.patch.object(
            optimize.subprocess,
            "check_output",
            return_value="3.54.1 full baseline version\n",
        ):
            with self.assertRaisesRegex(ValueError, "differs from the bundled SQLite"):
                optimize.check_sqlite_version(
                    dict(self.metadata, bundled_sqlite="3.55.0")
                )

    def test_missing_version_metadata_is_a_descriptive_error(self):
        for metadata in (
            {"inspection_sqlite": "3.54.1 full baseline version"},
            {"bundled_sqlite": "3.54.1"},
        ):
            with self.subTest(metadata=metadata):
                with (
                    mock.patch.object(optimize.subprocess, "check_output") as version,
                    self.assertRaisesRegex(
                        ValueError, "index_baseline.py --sqlite-version"
                    ),
                ):
                    optimize.check_sqlite_version(metadata)
                version.assert_not_called()


class McpTransport(unittest.TestCase):
    def test_coalesced_notification_and_response_use_one_readiness_wait(self):
        read_fd, write_fd = os.pipe()
        notification = b'{"jsonrpc":"2.0","method":"progress"}\n'
        response = b'{"jsonrpc":"2.0","id":1,"result":{}}\n'
        os.write(write_fd, notification + response)
        os.close(write_fd)
        with os.fdopen(read_fd, "rb", buffering=0) as stream:
            reader = search_baseline.BufferedLineReader(stream)
            deadline = time.monotonic() + 1
            with mock.patch.object(
                search_baseline.select, "select", wraps=search_baseline.select.select
            ) as ready:
                self.assertEqual(reader.readline(deadline), notification)
                self.assertEqual(reader.readline(deadline), response)
        self.assertEqual(ready.call_count, 1)

    def test_fragmented_line_is_accumulated(self):
        stream = mock.Mock()
        stream.fileno.return_value = 10
        reader = search_baseline.BufferedLineReader(stream)
        with (
            mock.patch.object(
                search_baseline.select, "select", return_value=([stream], [], [])
            ),
            mock.patch.object(
                search_baseline.os, "read", side_effect=[b'{"id":', b"1}\n"]
            ) as read,
        ):
            self.assertEqual(reader.readline(time.monotonic() + 1), b'{"id":1}\n')
        self.assertEqual(read.call_args_list, [mock.call(10, 65536)] * 2)

    def test_readline_uses_an_absolute_deadline(self):
        stream = mock.Mock()
        stream.fileno.return_value = 10
        reader = search_baseline.BufferedLineReader(stream)
        with (
            mock.patch.object(search_baseline.time, "monotonic", return_value=99.25),
            mock.patch.object(
                search_baseline.select, "select", return_value=([], [], [])
            ) as ready,
        ):
            with self.assertRaises(TimeoutError):
                reader.readline(100.0)
        ready.assert_called_once_with([stream], [], [], 0.75)

    def test_write_all_handles_partial_writes_and_flushes_once(self):
        class PartialWriter:
            def __init__(self):
                self.data = bytearray()
                self.flushes = 0

            def write(self, data):
                count = min(3, len(data))
                self.data.extend(data[:count])
                return count

            def flush(self):
                self.flushes += 1

        writer = PartialWriter()
        search_baseline.write_all(writer, b"abcdefghij")
        self.assertEqual(writer.data, b"abcdefghij")
        self.assertEqual(writer.flushes, 1)


class IndexBaselinePreflight(unittest.TestCase):
    def test_sqlite_mismatch_leaves_run_untouched(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / "run"
            run.mkdir()
            checkout_root = root / "checkouts"
            for name in ("linux", "kubernetes", "keycloak"):
                (checkout_root / name).mkdir(parents=True)
            helper = root / "sqlite-version"
            helper.touch()
            (root / "bin").mkdir()
            for name in ("codecite", "codecite-bench"):
                (root / "bin" / name).touch()
            argv = [
                "index_baseline.py",
                str(run),
                "--bin-dir",
                str(root / "bin"),
                "--checkout-root",
                str(checkout_root),
                "--sqlite-version",
                str(helper),
            ]

            def output(*args, **kwargs):
                if args[:2] == ("sqlite3", "-version"):
                    return "3.50.0 different CLI"
                if args[0] == helper:
                    return "3.54.1"
                raise AssertionError(f"unexpected command after failed preflight: {args}")

            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch.object(index_baseline, "output", side_effect=output),
                mock.patch.object(index_baseline, "check_checkout"),
            ):
                with self.assertRaisesRegex(ValueError, "differs from the bundled"):
                    index_baseline.main()
            self.assertEqual(list(run.iterdir()), [])

    def _argv(self, root, run):
        checkout_root = root / "checkouts"
        for name in ("linux", "kubernetes", "keycloak"):
            (checkout_root / name).mkdir(parents=True, exist_ok=True)
        helper = root / "sqlite-version"
        helper.touch()
        (root / "bin").mkdir(exist_ok=True)
        return [
            "index_baseline.py",
            str(run),
            "--bin-dir",
            str(root / "bin"),
            "--checkout-root",
            str(checkout_root),
            "--sqlite-version",
            str(helper),
        ], helper

    def test_bad_checkout_leaves_run_untouched(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / "run"
            run.mkdir()
            argv, helper = self._argv(root, run)
            for name in ("codecite", "codecite-bench"):
                (root / "bin" / name).touch()

            def output(*args, **kwargs):
                if args[:2] == ("sqlite3", "-version"):
                    return "3.54.1 CLI"
                if args[0] == helper:
                    return "3.54.1"
                raise AssertionError(f"unexpected command: {args}")

            def check_checkout(path, commit):
                if path.name == "keycloak":
                    raise ValueError(f"{path} is not clean")

            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch.object(index_baseline, "output", side_effect=output),
                mock.patch.object(
                    index_baseline, "check_checkout", side_effect=check_checkout
                ),
            ):
                with self.assertRaisesRegex(ValueError, "not clean"):
                    index_baseline.main()
            self.assertEqual(list(run.iterdir()), [])

    def test_missing_binary_leaves_run_untouched(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / "run"
            run.mkdir()
            argv, _ = self._argv(root, run)
            (root / "bin" / "codecite").touch()
            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch.object(index_baseline, "output", return_value="3.54.1"),
                mock.patch.object(index_baseline, "check_checkout"),
            ):
                with self.assertRaises(FileNotFoundError):
                    index_baseline.main()
            self.assertEqual(list(run.iterdir()), [])

    def test_run_pair_sets_private_data_and_cache_homes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = root / "data/corpus-1"
            data.parent.mkdir()
            cache = root / "cache"
            raw = root / "index.jsonl"
            raw.touch()
            manifest = root / "corpus.toml"
            manifest.write_text("name = 'corpus'\n")
            seen = {}

            def run(command, **kwargs):
                seen.update(kwargs["env"])
                (data / "time.txt").write_text("1.0 2\n")
                return subprocess.CompletedProcess(command, 1, stdout="", stderr="")

            with (
                mock.patch.object(index_baseline.subprocess, "run", side_effect=run),
                self.assertRaises(subprocess.CalledProcessError),
            ):
                index_baseline.run_pair(
                    root / "codecite", "corpus", manifest, data, cache, raw
                )
            self.assertEqual(seen["XDG_DATA_HOME"], str(data))
            self.assertEqual(seen["XDG_CACHE_HOME"], str(cache))


class SourceWalGuard(unittest.TestCase):
    def test_missing_and_empty_wal_files_are_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            db = Path(directory) / "corpus.db"
            db.touch()
            optimize.reject_uncheckpointed_wal(db)
            Path(str(db) + "-wal").touch()
            optimize.reject_uncheckpointed_wal(db)

    def test_nonempty_wal_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            db = Path(directory) / "corpus.db"
            db.touch()
            Path(str(db) + "-wal").write_bytes(b"uncheckpointed")
            with self.assertRaisesRegex(ValueError, "source has uncheckpointed WAL"):
                optimize.reject_uncheckpointed_wal(db)

    def test_main_rejects_wal_before_copying(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline = root / "baseline"
            db = baseline / "data/corpus-1/codecite/corpus.db"
            db.parent.mkdir(parents=True)
            db.touch()
            Path(str(db) + "-wal").write_bytes(b"uncheckpointed")
            (baseline / "index-metadata.json").write_text("{}")
            output = root / "output"
            argv = ["optimize.py", str(baseline), str(output), "corpus"]
            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch.object(optimize, "check_sqlite_version"),
                mock.patch.object(optimize.shutil, "copyfile") as copyfile,
            ):
                with self.assertRaisesRegex(ValueError, "uncheckpointed WAL"):
                    optimize.main()
            copyfile.assert_not_called()
            self.assertFalse(output.exists())

    def test_all_source_wals_are_checked_before_any_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline = root / "baseline"
            for corpus in ("clean", "dirty"):
                db = baseline / f"data/{corpus}-1/codecite/{corpus}.db"
                db.parent.mkdir(parents=True)
                db.touch()
            dirty = baseline / "data/dirty-1/codecite/dirty.db"
            Path(str(dirty) + "-wal").write_bytes(b"uncheckpointed")
            (baseline / "index-metadata.json").write_text("{}")
            output = root / "output"
            argv = [
                "optimize.py",
                str(baseline),
                str(output),
                "clean",
                "dirty",
            ]
            process = mock.Mock()
            process.poll.return_value = 0
            process.communicate.return_value = ("", "")
            process.returncode = 0
            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch.object(optimize, "check_sqlite_version"),
                mock.patch.object(optimize, "stats", return_value={"chunks": 0}),
                mock.patch.object(optimize.subprocess, "Popen", return_value=process),
                mock.patch.object(optimize.shutil, "copyfile") as copyfile,
            ):
                with self.assertRaisesRegex(ValueError, "uncheckpointed WAL"):
                    optimize.main()
            copyfile.assert_not_called()
            self.assertFalse(output.exists())


class SearchBaselinePreflight(unittest.TestCase):
    def test_stratum_violation_leaves_no_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            run = Path(directory)
            binary = run / "codecite"
            binary.write_bytes(b"binary")
            sha = hashlib.sha256(b"binary").hexdigest()
            (run / "index-metadata.json").write_text(
                json.dumps({"binary_sha256": {"codecite": sha}})
            )
            labels = run / "queries.json"
            query = {"id": "r1", "class": "rare-term", "text": "rare", "answers": []}
            labels.write_text(
                json.dumps({"corpora": {"first": [], "second": [query]}})
            )
            argv = ["search_baseline.py", str(run), "--binary", str(binary)]
            before = sorted(run.iterdir())
            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch.object(search_baseline, "LABELS", labels),
                mock.patch.object(
                    search_baseline.subprocess, "check_output", return_value="0\n"
                ),
                mock.patch.object(search_baseline.subprocess, "Popen") as popen,
            ):
                with self.assertRaisesRegex(ValueError, "stratum violated"):
                    search_baseline.main()
            popen.assert_not_called()
            self.assertEqual(sorted(run.iterdir()), before)


class IncrementalOptimizePreflight(unittest.TestCase):
    def test_missing_helper_leaves_no_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline = root / "baseline"
            source = baseline / "data/identity-bench-1/codecite/identity-bench.db"
            source.parent.mkdir(parents=True)
            source.touch()
            bin_dir = root / "bin"
            bin_dir.mkdir()
            hashes = {}
            for name in ("codecite", "codecite-bench"):
                (bin_dir / name).write_bytes(name.encode())
                hashes[name] = hashlib.sha256(name.encode()).hexdigest()
            manifest = incremental_optimize.ROOT / "evals/retrieval/identity-bench.toml"
            (baseline / "index-metadata.json").write_text(json.dumps({
                "binary_sha256": hashes,
                "manifest_sha256": {"identity-bench": incremental_optimize.sha(manifest)},
            }))
            output = root / "output"
            argv = [
                "incremental_optimize.py",
                str(baseline),
                str(output),
                "--helper",
                str(root / "missing-helper"),
                "--bin-dir",
                str(bin_dir),
            ]
            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch.object(
                    incremental_optimize.subprocess,
                    "check_output",
                    return_value='{"outcomes": [], "all": {}}',
                ),
            ):
                with self.assertRaises(FileNotFoundError):
                    incremental_optimize.main()
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()

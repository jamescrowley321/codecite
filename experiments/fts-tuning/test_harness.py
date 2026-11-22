"""Tests for the FTS tuning harness. Run: python3 -m unittest experiments/fts-tuning/test_harness.py"""

import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import optimize  # noqa: E402


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
            argv = ["optimize.py", str(baseline), str(root / "output"), "corpus"]
            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch.object(optimize, "check_sqlite_version"),
                mock.patch.object(optimize.shutil, "copyfile") as copyfile,
            ):
                with self.assertRaisesRegex(ValueError, "uncheckpointed WAL"):
                    optimize.main()
            copyfile.assert_not_called()


if __name__ == "__main__":
    unittest.main()

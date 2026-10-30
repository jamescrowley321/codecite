"""The history miner's selection and output, without the network."""

import tomllib

from codecite_evals.history import Change, Mined, PullRequest, issue_text, queries_toml, select_prs, manifest_toml


def pr(number, *changes, author="jamescrowley321", parent=None, issue=None):
    return PullRequest(
        number=number,
        title=f"feat: change {number}",
        url=f"https://github.com/o/r/pull/{number}",
        author=author,
        parent=parent or f"p{number}",
        changes=list(changes),
        issue=issue,
    )


def everything_exists(commit, path):
    return True


def test_bots_and_lockfile_only_prs_are_left_out():
    prs = [
        pr(5, Change("M", "src/a.py")),
        pr(4, Change("M", "src/a.py"), author="dependabot[bot]"),
        pr(3, Change("M", "uv.lock"), Change("M", "CHANGELOG.md")),
        pr(2, Change("M", "src/b.py"), author="renovate"),
        pr(6, Change("M", "src/b.py"), author="app/dependabot"),
        pr(1, Change("M", "src/c.py")),
    ]
    _, mined = select_prs(prs, everything_exists, 40)
    assert [m.number for m in mined] == [5, 1]


def test_added_and_deleted_files_are_no_answer():
    prs = [pr(2, Change("A", "src/new.py"), Change("D", "src/old.py")), pr(1, Change("M", "src/a.py"))]
    _, mined = select_prs(prs, everything_exists, 40)
    assert [m.number for m in mined] == [1]


def test_the_pinned_commit_is_the_oldest_samples_parent():
    prs = [pr(3, Change("M", "a/x.py"), parent="ccc"), pr(2, Change("M", "a/y.py"), parent="bbb")]
    commit, _ = select_prs(prs, everything_exists, 40)
    assert commit == "bbb"


def test_a_renamed_file_answers_with_its_old_path():
    prs = [pr(1, Change("R", "src/new_name.py", old_path="src/old_name.py"))]
    _, mined = select_prs(prs, everything_exists, 40)
    assert mined[0].answers == ["src/old_name.py"]


def test_a_file_missing_at_the_pinned_commit_is_dropped():
    prs = [
        pr(2, Change("M", "src/later.py"), Change("M", "src/a.py")),
        pr(1, Change("M", "src/a.py"), parent="base"),
    ]
    _, mined = select_prs(prs, lambda commit, path: path != "src/later.py", 40)
    assert mined[0].answers == ["src/a.py"]


def test_a_pr_with_no_answer_left_at_the_pinned_commit_is_dropped():
    prs = [pr(2, Change("M", "src/later.py")), pr(1, Change("M", "src/a.py"))]
    _, mined = select_prs(prs, lambda commit, path: path != "src/later.py", 40)
    assert [m.number for m in mined] == [1]


def test_a_path_the_bench_cannot_label_is_dropped():
    # codecite-bench reads a label with no `/` or `.` as a malformed spec label.
    prs = [pr(1, Change("M", "Makefile"), Change("M", "src/a.py"))]
    _, mined = select_prs(prs, everything_exists, 40)
    assert mined[0].answers == ["src/a.py"]


def test_max_keeps_the_newest():
    prs = [pr(n, Change("M", "src/a.py")) for n in (9, 8, 7, 6)]
    commit, mined = select_prs(prs, everything_exists, 2)
    assert [m.number for m in mined] == [9, 8]
    assert commit == "p8"


def test_issue_text_is_title_and_first_paragraph_capped_at_400():
    body = "## Why\n\n<!-- template -->\n\nThe token endpoint accepts credentials\nin the query string.\n\nMore detail."
    assert issue_text("Fix client auth", body) == (
        "Fix client auth\n\nThe token endpoint accepts credentials in the query string."
    )
    long = issue_text("T", "word " * 200)
    assert len(long) <= 400
    assert not long.endswith(" ")
    assert long.split()[-1] == "word"
    assert issue_text("Only a title", "") == "Only a title"
    # A window with no space to cut at is cut at the limit.
    assert len(issue_text("T", "x" * 1000)) == 400


def test_written_toml_round_trips_special_characters():
    title = 'fix: quote "x", a \\ backslash\tand\nnewline'
    mined = [Mined(1, "https://github.com/o/r/pull/1", title, title, ["src/a.py"])]
    data = tomllib.loads(queries_toml("o/r", "abc123", mined, "2026-10-29"))
    assert data["corpus"] == "history-r"
    assert [q["id"] for q in data["query"]] == ["r-pr1-title", "r-pr1-issue"]
    assert data["query"][0]["text"] == title
    assert data["query"][0]["class"] == "history-title"
    assert data["query"][1]["class"] == "history-issue"
    assert data["query"][0]["answers"] == ["src/a.py"]
    assert data["query"][0]["source"] == "https://github.com/o/r/pull/1"


def test_a_pr_without_an_issue_gets_only_a_title_query():
    mined = [Mined(1, "u", "t", None, ["src/a.py"])]
    data = tomllib.loads(queries_toml("o/r", "abc", mined, "2026-10-29"))
    assert [q["class"] for q in data["query"]] == ["history-title"]


def test_the_manifest_pins_one_git_source():
    data = tomllib.loads(manifest_toml("o/r", "abc123", "MIT"))
    assert data["name"] == "history-r"
    assert data["source"] == [
        {"kind": "git", "name": "r", "url": "https://github.com/o/r", "commit": "abc123", "license": "MIT", "use": "adapt"}
    ]
    assert "license" not in tomllib.loads(manifest_toml("o/r", "abc123", None))["source"][0]


def bench(engine, title, issue):
    def scores(r10, n):
        return {"queries": n, "recall_at_1": r10 / 2, "recall_at_10": r10, "mrr": r10 / 3}

    return {"engine": engine, "classes": {"history-issue": scores(*issue), "history-title": scores(*title)}}


def test_the_scorecard_has_a_row_per_repository_class_and_engine():
    from codecite_evals.history import scorecard

    runs = {
        "r": {
            "codecite": bench("codecite", (0.5, 20), (0.25, 8)),
            "grep": bench("grep", (0.75, 20), (0.5, 8)),
            "index": "chunks: 1200 written · elapsed: 2.10s",
        },
        "s": {"failed": "error: commit not found"},
    }
    table = scorecard(runs)
    assert "| r | history-title | codecite | 20 | 0.25 | 0.50 | 0.17 |" in table
    assert "| r | history-title | grep | 20 | 0.38 | 0.75 | 0.25 |" in table
    assert "| r | history-issue | codecite | 8 | 0.12 | 0.25 | 0.08 |" in table
    assert "| s | failed: error: commit not found |" in table
    assert "chunks: 1200 written · elapsed: 2.10s" in table


def test_a_pr_touching_more_than_20_files_is_left_out():
    wide = pr(2, *[Change("M", f"src/f{i}.py") for i in range(21)])
    twenty = pr(1, *[Change("M", f"src/g{i}.py") for i in range(20)])
    _, mined = select_prs([wide, twenty], everything_exists, 40)
    assert [m.number for m in mined] == [1]


def test_release_prs_are_left_out():
    prs = [
        pr(3, Change("M", "src/version.txt")),
        pr(2, Change("M", "src/a.py")),
        pr(1, Change("M", "src/b.py")),
    ]
    prs[0].title = "chore(main): release 3.12.1"
    prs[1].title = "chore: release v2"
    _, mined = select_prs(prs, everything_exists, 40)
    assert [m.number for m in mined] == [1]


def test_files_more_than_a_quarter_of_the_sample_changed_are_no_answer():
    # docs/design.md is in 3 of 8 sampled PRs (37%); src/x.py in 2 (25%).
    prs = [pr(n, Change("M", "docs/design.md"), Change("M", f"src/only{n}.py")) for n in (8, 7, 6)]
    prs += [pr(n, Change("M", "src/x.py")) for n in (5, 4)]
    prs += [pr(n, Change("M", f"src/only{n}.py")) for n in (3, 2, 1)]
    _, mined = select_prs(prs, everything_exists, 40)
    answers = {m.number: m.answers for m in mined}
    assert answers[8] == ["src/only8.py"]
    assert answers[5] == ["src/x.py"]


def test_a_pr_whose_only_answer_is_a_hub_file_is_dropped():
    prs = [pr(n, Change("M", "docs/design.md")) for n in (6, 5, 4)]
    prs += [pr(n, Change("M", f"src/only{n}.py")) for n in (3, 2, 1)]
    _, mined = select_prs(prs, everything_exists, 40)
    assert [m.number for m in mined] == [3, 2, 1]


def test_a_file_in_two_of_a_small_sample_is_no_hub():
    prs = [pr(n, Change("M", "src/a.py")) for n in (2, 1)]
    _, mined = select_prs(prs, everything_exists, 40)
    assert [m.number for m in mined] == [2, 1]


def test_the_scorecard_shows_the_popular_floor_and_a_pooled_row():
    from codecite_evals.history import scorecard

    run = {
        "codecite": {**bench("codecite", (0.5, 20), (0.25, 8)), "all": {"queries": 28, "recall_at_1": 0.2, "recall_at_10": 0.43, "mrr": 0.3}},
        "popular": bench("popular", (0.1, 20), (0.1, 8)),
    }
    table = scorecard({"r": run})
    assert "| r | history-title | popular | 20 | 0.05 | 0.10 | 0.03 |" in table
    assert "| r | all | codecite | 28 | 0.20 | 0.43 | 0.30 |" in table
    assert "grep" not in table


def test_a_missing_or_unreadable_report_does_not_hide_the_others(tmp_path):
    import json
    from codecite_evals.history import scorecard_main

    (tmp_path / "a-codecite.json").write_text(json.dumps(bench("codecite", (0.5, 4), (0.5, 2))))
    (tmp_path / "b-codecite.json").write_text("")
    (tmp_path / "b-failed.txt").write_text("codecite-bench failed\n")
    scorecard_main([str(tmp_path)])
    text = (tmp_path / "scorecard.md").read_text()
    assert "| a | history-title | codecite | 4 |" in text
    assert "| b | failed: codecite-bench failed |" in text


def test_a_path_holding_a_hash_is_no_answer():
    # codecite-bench splits a label at `#`, so such a path can't be labelled.
    prs = [pr(1, Change("M", "docs/c#/notes.md"), Change("M", "src/a.py"))]
    _, mined = select_prs(prs, everything_exists, 40)
    assert mined[0].answers == ["src/a.py"]


def test_a_heading_line_right_above_the_text_does_not_hide_it():
    assert issue_text("T", "## Problem\nThe token endpoint fails.") == "T\n\nThe token endpoint fails."
    assert issue_text("T", "<!-- template\nlines -->\n\n#123 broke login") == "T\n\n#123 broke login"


def test_the_closing_issue_must_be_in_the_same_owner_and_repository():
    from codecite_evals.history import closing_issue

    def ref(number, owner, name):
        return {"number": number, "repository": {"name": name, "owner": {"login": owner}}}

    assert closing_issue([ref(4, "other", "r"), ref(5, "o", "r")], "o/r") == 5
    assert closing_issue([ref(4, "other", "r")], "o/r") is None
    assert closing_issue([], "o/r") is None


def test_name_status_paths_are_read_raw_from_nul_separated_output():
    from codecite_evals.history import parse_name_status

    out = "M\0src/café.py\0R100\0old\tname.py\0new.py\0A\0x.py\0"
    assert parse_name_status(out) == [
        Change("M", "src/café.py"),
        Change("R", "new.py", old_path="old\tname.py"),
        Change("A", "x.py"),
    ]


def test_a_rebase_merged_pr_spans_all_its_commits():
    from codecite_evals.history import merged_span

    assert merged_span(2, "Merge pull request #7 from o/b", 7, lambda: 9) == 1
    assert merged_span(1, "feat: add x (#7)", 7, lambda: 9) == 1
    # A rebase merge: the merge commit is the PR's last commit of several.
    assert merged_span(1, "feat: add x", 7, lambda: 3) == 3
    assert merged_span(1, "feat: add x (#70)", 7, lambda: 1) == 1


def cost_report(queries, ours, lines, files):
    def c(v):
        return {"median_bytes": v[0], "median_ms": v[1], "p90_ms": v[2]}

    return {"engine": "cost", "queries": queries, "codecite": c(ours), "rg_lines": c(lines), "rg_files": c(files)}


def test_the_scorecard_shows_what_a_query_costs_next_to_rg():
    from codecite_evals.history import scorecard

    run = {
        "codecite": bench("codecite", (0.5, 20), (0.25, 8)),
        "cost": cost_report(28, (1200, 41.5, 60.2), (90000, 12.0, 30.0), (800, 11.0, 29.5)),
    }
    table = scorecard({"r": run, "s": {"codecite": bench("codecite", (0.5, 4), (0.5, 2))}})
    assert "## Cost per query" in table
    assert "| Repository | Queries | codecite bytes | codecite ms | rg -n bytes | rg -n ms | rg -l bytes | rg -l ms |" in table
    assert "| r | 28 | 1200 | 41.5 (60.2) | 90000 | 12.0 (30.0) | 800 | 11.0 (29.5) |" in table
    assert "| s |" not in table.split("## Cost per query")[1]


def test_without_a_cost_report_the_scorecard_has_no_cost_table():
    from codecite_evals.history import scorecard

    assert "Cost per query" not in scorecard({"r": {"codecite": bench("codecite", (0.5, 20), (0.25, 8))}})


def test_a_cost_report_is_read_from_the_results_directory(tmp_path):
    import json
    from codecite_evals.history import scorecard_main

    (tmp_path / "a-codecite.json").write_text(json.dumps(bench("codecite", (0.5, 4), (0.5, 2))))
    (tmp_path / "a-cost.json").write_text(json.dumps(cost_report(6, (10, 1.0, 2.0), (20, 3.0, 4.0), (5, 6.0, 7.0))))
    scorecard_main([str(tmp_path)])
    assert "| a | 6 | 10 | 1.0 (2.0) | 20 | 3.0 (4.0) | 5 | 6.0 (7.0) |" in (tmp_path / "scorecard.md").read_text()


def test_a_failed_cost_step_keeps_the_recall_rows():
    from codecite_evals.history import scorecard

    run = {"codecite": bench("codecite", (0.5, 20), (0.25, 8)), "cost_failed": "codecite-bench failed for cost"}
    table = scorecard({"r": run})
    recall, cost = table.split("## Cost per query")
    assert "| r | history-title | codecite | 20 | 0.25 | 0.50 | 0.17 |" in recall
    assert "failed" not in recall
    assert "| r | failed: codecite-bench failed for cost |" in cost


def test_a_cost_failure_is_read_from_the_results_directory(tmp_path):
    import json
    from codecite_evals.history import scorecard_main

    (tmp_path / "a-codecite.json").write_text(json.dumps(bench("codecite", (0.5, 4), (0.5, 2))))
    (tmp_path / "a-cost-failed.txt").write_text("cannot extract abc\n")
    scorecard_main([str(tmp_path)])
    text = (tmp_path / "scorecard.md").read_text()
    assert "| a | history-title | codecite | 4 |" in text
    assert "| a | failed: cannot extract abc |" in text.split("## Cost per query")[1]
    assert "a-cost" not in text


def test_run_sh_files_a_cost_failure_apart_from_the_recall_reports(tmp_path):
    """run.sh against stub binaries whose cost step fails: the recall
    reports stay, the failure lands in `<repo>-cost-failed.txt`, and a
    stale one from an earlier run is cleared first."""
    import os
    import shutil
    import subprocess
    from pathlib import Path

    here = tmp_path / "evals" / "history"
    (here / "demo").mkdir(parents=True)
    shutil.copy(Path(__file__).resolve().parents[1] / "history" / "run.sh", here / "run.sh")
    clones, repo = tmp_path / "clones", tmp_path / "clones" / "demo"
    repo.mkdir(parents=True)
    git = ["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"]
    subprocess.run([*git, "init", "-q"], check=True)
    (repo / "a.py").write_text("x = 1\n")
    subprocess.run([*git, "add", "."], check=True)
    subprocess.run([*git, "commit", "-q", "-m", "init"], check=True)
    commit = subprocess.run([*git, "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
    (here / "demo" / "manifest.toml").write_text(f'url = "{repo}"\ncommit = "{commit}"\n')
    (here / "demo" / "queries.toml").write_text("")
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "codecite").write_text("#!/bin/sh\nprintf 'chunks: 1 written\\nelapsed: 0.01s\\n'\n")
    (bin_dir / "codecite-bench").write_text(
        '#!/bin/sh\nfor a in "$@"; do [ "$a" = --cost ] && exit 1; done\necho "{}"\n'
    )
    for stub in bin_dir.iterdir():
        stub.chmod(0o755)
    results = here / "results"
    results.mkdir()
    (results / "demo-cost-failed.txt").write_text("stale\n")
    env = {**os.environ, "CODECITE_BIN": str(bin_dir), "HISTORY_CLONES": str(clones)}
    subprocess.run(["bash", str(here / "run.sh"), "demo"], env=env, capture_output=True)
    for kind in ("codecite", "grep", "popular"):
        assert (results / f"demo-{kind}.json").exists(), kind
    assert not (results / "demo-cost.json").exists()
    assert not (results / "demo-failed.txt").exists()
    assert (results / "demo-cost-failed.txt").read_text() == "codecite-bench failed for cost\n"

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

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

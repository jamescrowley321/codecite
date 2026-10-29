"""The log scrub deletes what holds the token, in any form, and anything it can't read."""

import base64
import urllib.parse
import zipfile

import pytest

from codecite_evals.scrub import scrub

TOKEN = "sk-ant-oat01-" + "Ab3_x9-QrT7zLm2Kp8Vn4Ws6Yc1Hd5Fg0Je" * 2


def eval_log(path, text):
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as z:
        z.writestr("header.json", "{}")
        z.writestr("samples/1.json", text)
    return path


def test_a_clean_log_stays(tmp_path):
    eval_log(tmp_path / "a.eval", "nothing to see")
    (tmp_path / "scorecard.md").write_text("| model |")
    assert scrub(tmp_path, TOKEN) == []
    assert {p.name for p in tmp_path.iterdir()} == {"a.eval", "scorecard.md"}


@pytest.mark.parametrize(
    "form",
    [
        TOKEN,
        TOKEN[20:60],  # cut, as a partial echo would be
        base64.b64encode(f"CLAUDE_CODE_OAUTH_TOKEN={TOKEN}\n".encode()).decode(),
        base64.b64encode(f"x{TOKEN}".encode()).decode(),
        urllib.parse.quote(f"t={TOKEN}"),
    ],
)
def test_a_log_holding_the_token_in_any_form_goes(tmp_path, form):
    eval_log(tmp_path / "a.eval", f"$ env\nPATH=/bin\n{form}\n")
    (removed, why), = scrub(tmp_path, TOKEN)
    assert removed.name == "a.eval" and why == "it held the token"
    assert not removed.exists()


def test_a_plain_file_holding_it_goes(tmp_path):
    (tmp_path / "scorecard.md").write_text(f"sandbox failed: {TOKEN}")
    assert [p.name for p, _ in scrub(tmp_path, TOKEN)] == ["scorecard.md"]


def test_a_cut_off_eval_goes_unread(tmp_path):
    # A log cut short mid-write has no central directory: it can't be read
    # through, so it can't be cleared.
    whole = eval_log(tmp_path / "whole.eval", "nothing").read_bytes()
    (tmp_path / "cut.eval").write_bytes(whole[: len(whole) // 2])
    (path, why), = scrub(tmp_path, TOKEN)
    assert path.name == "cut.eval" and "couldn't be read through" in why


def test_a_corrupt_member_goes_and_the_rest_are_still_checked(tmp_path):
    path = eval_log(tmp_path / "a.eval", "nothing to see here " * 50)
    data = bytearray(path.read_bytes())
    data[60:80] = b"\xff" * 20  # inside the first member's compressed data
    path.write_bytes(bytes(data))
    eval_log(tmp_path / "b.eval", TOKEN)
    eval_log(tmp_path / "c.eval", "clean")
    assert sorted(p.name for p, _ in scrub(tmp_path, TOKEN)) == ["a.eval", "b.eval"]
    assert (tmp_path / "c.eval").exists()

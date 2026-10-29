"""Delete every eval log that holds the Claude token, before CI uploads them.

    CLAUDE_CODE_OAUTH_TOKEN=... python -m codecite_evals.scrub <log dir>

The agent runs as root in the sandbox with the token in its environment, and
its output reaches the logs, so this is the backstop before the logs become a
public artifact. It fails closed: a file it can't read through, an `.eval`
that isn't a whole zip, or any error deletes the file. It looks for any run
of 31 or more characters of the token past its shared prefix, or of the same
reversed, in hex, in base64 at any alignment or URL-encoded, both in the text
and in the text with its whitespace and `\\n`-style escapes taken out (so a
line-wrapped `base64` passes no better). It can't catch a deliberately
transformed copy (compressed, encrypted, split up), which is why the agent's
only way out is the proxy.
Exit 1 when it deleted anything, so the run is flagged.
"""

import base64
import os
import re
import sys
import urllib.parse
import zipfile
from pathlib import Path

if sys.version_info < (3, 14):
    import zipfile_zstd  # noqa: F401  (zstd in zipfile, as Inspect does)

# The token's shared prefix, which says nothing about the token.
PREFIX = b"sk-ant-oat01-"
# The length of the needles: any copy of 2 * RUN - 1 or more characters of a
# form holds one of them, as they start every RUN characters.
RUN = 16
# What a log may break an encoded token with: whitespace, and the escapes a
# JSON string writes for it.
BREAKS = re.compile(rb"\s+|\\[nrt]")


def needles(token: str) -> set[bytes]:
    """The runs of the token's forms a log must not hold."""
    body = token.encode().removeprefix(PREFIX)
    forms = [body, body[::-1], body.hex().encode(), body.hex().upper().encode()]
    forms.append(urllib.parse.quote(body.decode(), safe="").encode())
    # Base64 at each of the three alignments the body can start at, without
    # the characters its neighbours change.
    forms += [base64.b64encode(b"\0" * shift + body)[4:-4] for shift in range(3)]
    found = set()
    for form in forms:
        found.update(form[i : i + RUN] for i in range(0, len(form) - RUN + 1, RUN))
    return found


def found(data: bytes, forms: set[bytes]) -> bool:
    """Whether the data holds any form, as it is or with its breaks taken out."""
    return any(form in chunk for chunk in (data, BREAKS.sub(b"", data)) for form in forms)


def holds(path: Path, forms: set[bytes]) -> bool:
    """Whether the file holds any form, reading through a zip's members.
    Raises when it can't read the whole file."""
    if path.suffix == ".eval" and not zipfile.is_zipfile(path):
        raise ValueError("not a whole zip")
    chunks = []
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as z:
            chunks = [z.read(name) for name in z.namelist()]
    chunks.append(path.read_bytes())
    return any(found(chunk, forms) for chunk in chunks)


def scrub(root: Path, token: str) -> list[tuple[Path, str]]:
    """Delete each file under root that holds the token or can't be read
    through; return what went, and why."""
    forms, removed = needles(token), []
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        try:
            why = "it held the token" if holds(path, forms) else None
        except Exception as e:
            why = f"it couldn't be read through ({type(e).__name__})"
        if why:
            path.unlink()
            removed.append((path, why))
    return removed


def main() -> None:
    token = os.environ.get("CLAUDE_CODE_OAUTH_TOKEN", "")
    if len(sys.argv) != 2 or not token:
        sys.exit("usage: CLAUDE_CODE_OAUTH_TOKEN=... python -m codecite_evals.scrub <log dir>")
    if not needles(token):
        sys.exit(f"{len(token)} characters is too short to be a Claude token")
    removed = scrub(Path(sys.argv[1]), token)
    for path, why in removed:
        print(f"::error::removed {path}: {why}")
    sys.exit(1 if removed else 0)


if __name__ == "__main__":
    main()

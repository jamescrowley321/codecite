"""Delete every eval log that holds the Claude token, before CI uploads them.

    CLAUDE_CODE_OAUTH_TOKEN=... python -m codecite_evals.scrub <log dir>

Claude Code strips the token from its commands' environment, but the agent
runs as root in the sandbox and its output reaches the logs, so this is the
backstop before the logs become a public artifact. It fails closed: a file it
can't read through, an `.eval` that isn't a whole zip, or any error deletes
the file. It looks for the token itself, a distinctive run of its characters
(to catch a cut or partly echoed copy), and its base64 and URL-encoded forms.
Exit 1 when it deleted anything, so the run is flagged.
"""

import base64
import os
import sys
import urllib.parse
import zipfile
from pathlib import Path

if sys.version_info < (3, 14):
    import zipfile_zstd  # noqa: F401  (zstd in zipfile, as Inspect does)

# The length of the runs of the token looked for, past any shared prefix.
RUN = 16


def needles(token: str) -> set[bytes]:
    """The forms of the token a log must not hold."""
    raw = token.encode()
    found = {raw, urllib.parse.quote(token, safe="").encode()}
    # A run from the token's middle: its prefix (sk-ant-oat01-) is shared.
    middle = len(raw) // 2
    if len(raw) >= 2 * RUN:
        found.add(raw[middle - RUN // 2 : middle + RUN // 2])
    # Base64 of the token at each of the three alignments it can start at,
    # without the characters its neighbours change.
    for shift in range(3):
        encoded = base64.b64encode(b"\0" * shift + raw)[4:-4]
        if len(encoded) >= RUN:
            found.add(encoded)
    return found


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
    return any(form in chunk for chunk in chunks for form in forms)


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
    removed = scrub(Path(sys.argv[1]), token)
    for path, why in removed:
        print(f"::error::removed {path}: {why}")
    sys.exit(1 if removed else 0)


if __name__ == "__main__":
    main()

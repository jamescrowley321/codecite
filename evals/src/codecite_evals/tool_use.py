"""Shell command patterns for the tools measured in eval transcripts."""

import re

# A binary at the start of a command, or after a shell separator, with
# optional variable assignments and a path. Naming it as an argument is not a call.
_CLI = r"(?:^|[;&|(`\n]|\$\()\s*(?:\w+=\S*\s+)*(?:\S*/)?{}(?=\s|$)"
CLI = re.compile(_CLI.format("codecite"))
GRAPH_CLI = re.compile(_CLI.format("codegraph"))


def cli(tool: str) -> re.Pattern:
    """The shell pattern for one tool, not the other."""
    return {"codecite": CLI, "codegraph": GRAPH_CLI}[tool]

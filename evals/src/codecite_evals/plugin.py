"""The plugin arm: Claude Code with the codecite plugin (plugins/codecite:
skill, MCP server and session-start hook) loaded from PLUGIN_DIR with
--plugin-dir. codecite stays on PATH, since the plugin's MCP server and hook
run `codecite`, and CODECITE_CORPUS names the corpus they use. No note goes
in the prompt: the skill and the session note are the plugin's own.
"""

from codecite_evals import config

PLUGIN_DIR = "/opt/codecite-plugin"
CORPUS_ENV = "CODECITE_CORPUS"
CORPUS = "eval"
SOURCE = "plugins/codecite"


def files() -> dict[str, str]:
    """What the arm's sandbox gets, by path: every file of the plugin in the
    checkout, under PLUGIN_DIR."""
    root = config.ROOT / SOURCE
    return {f"{PLUGIN_DIR}/{path.relative_to(root).as_posix()}": path.read_text() for path in sorted(root.rglob("*")) if path.is_file()}

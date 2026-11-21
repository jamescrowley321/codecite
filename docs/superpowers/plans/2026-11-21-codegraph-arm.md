# A codegraph arm in the agent eval — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One eval run that measures baseline, codecite and CodeGraph against the same task, agent, model and host.

**Architecture:** A third arm, `codegraph`, joins `config.ARMS` under a new `tools` set. The arm reaches the sandbox as an MCP server plus a CLI on `PATH`, mirroring the codecite arm exactly; the image bakes CodeGraph and its `.codegraph/` indexes in at build time and every other arm strips them at sample time. `report.py` stops assuming two arms and compares every pair, gating only the codecite arm against the reference.

**Tech Stack:** Python 3.15 on the eval host (package compatibility: Python 3.12+; sandbox and frozen task interpreters unchanged), Inspect AI, pytest, Docker, `@colbymchenry/codegraph` 1.6.4 (npm, MIT).

**Spec:** `docs/superpowers/specs/2026-11-21-codegraph-arm-design.md`

**Issue:** #275

Implementation follows the actual Pi 1.2 `mcp.json` discovery and keeps the
assist binary off `PATH`. Cleanup failures must abort the sample. The tools
head-to-head requires adoption in both tool arms; the existing gate is unchanged.
The panel and workflow keep tools logs separate from the default A/B logs.

## Global Constraints

- CodeGraph version: `@colbymchenry/codegraph@1.6.4`, installed from npm, `codegraph telemetry off` before any index is built.
- CodeGraph's MCP command is `codegraph serve --mcp`; its CLI binary goes at `/opt/codegraph/bin/codegraph`.
- The sandbox has no route out but the proxy that lets through `api.anthropic.com` alone. Indexing happens at image build; a run reads only `.codegraph/`.
- No file under `evals/tasks/<id>/` changes: `codecite-eval-frozen` fails a PR that touches a frozen task's directory.
- The existing gate is unchanged: codecite against the reference arm still decides whether a PR blocks. Every codegraph comparison is reported, never gated.
- A prompt note describes its own tool only. Neither note names the other tool.
- Tests run with `cd evals && uv run --locked pytest`. Every task ends green on the whole suite, not just its own file.

## Review Focus

- An arm's prompt note differing in length from another's, which would let a note rather than a tool explain a result.
- A `.codegraph/` or `/opt/codegraph` left in a non-codegraph arm's sandbox, which silently gives that arm the tool under test.
- `used` counting the wrong binary, so a run that ignored its tool reads `no loss` instead of `not measured`.
- A comparison gated that should only be reported, or the codecite gate lost when a third arm joins the set.
- A scorecard row for an arm with no usable runs disappearing, when the verdict it carries is the point.

---

### Task 1: The `tools` arm set and the codegraph note

**Files:**
- Modify: `evals/src/codecite_evals/config.py:24-33`
- Modify: `evals/src/codecite_evals/task.py` (`codecite_note`, `prompt`)
- Test: `evals/tests/test_tools_arm.py` (create)

**Interfaces:**
- Produces: `config.ARMS["tools"] == (("baseline", "baseline"), ("codecite", "codecite"), ("codegraph", "codegraph"))`
- Produces: `task.corpus_words(task_dir: Path, corpus: str = "") -> str` — the words naming what a task's corpus holds, lifted verbatim out of `codecite_note`.
- Produces: `task.codegraph_note(task_dir: Path, corpus: str = "") -> str`
- Produces: `task.note(task_dir: Path, condition: str, corpus: str = "") -> str | None` — the note for `codecite` or `codegraph`, `None` for every other condition.

- [ ] **Step 1: Write the failing test**

Create `evals/tests/test_tools_arm.py`:

```python
"""The `tools` arm set: baseline, codecite and codegraph in one run."""

import json

from codecite_evals import config, task


def test_the_tools_arm_runs_three_arms():
    assert config.ARMS["tools"] == (
        ("baseline", "baseline"),
        ("codecite", "codecite"),
        ("codegraph", "codegraph"),
    )


def test_one_sample_per_arm_named_by_its_arm():
    dataset = task.fastapi_middleware(arms="tools").dataset
    assert [s.id for s in dataset] == ["baseline", "codecite", "codegraph"]
    assert [s.metadata["condition"] for s in dataset] == ["baseline", "codecite", "codegraph"]


def test_each_note_names_only_its_own_tool():
    prompts = {s.id: s.input for s in task.fastapi_middleware(arms="tools").dataset}
    assert "codecite" in prompts["codecite"]
    assert "codegraph" not in prompts["codecite"].lower()
    assert "codegraph" in prompts["codegraph"]
    assert "codecite" not in prompts["codegraph"].lower()


def test_the_baseline_prompt_carries_no_note():
    prompts = {s.id: s.input for s in task.fastapi_middleware(arms="tools").dataset}
    prompt = (config.task_dir("fastapi-middleware") / "prompt.md").read_text()
    assert prompts["baseline"] == prompt.replace("{identity_model}", task.IDENTITY_MODEL)


def test_the_two_notes_are_of_a_length():
    """A note longer than the other would let note length, not the tool,
    explain a result: within a quarter of each other's length."""
    d = config.task_dir("fastapi-middleware")
    codecite, codegraph = task.codecite_note(d), task.codegraph_note(d)
    assert abs(len(codegraph) - len(codecite)) <= len(codecite) / 4


def test_only_the_two_tool_arms_get_a_note():
    d = config.task_dir("fastapi-middleware")
    assert task.note(d, "baseline") is None
    assert task.note(d, "assist") is None
    assert task.note(d, "codecite") == task.codecite_note(d)
    assert task.note(d, "codegraph") == task.codegraph_note(d)
```

- [ ] **Step 2: Run it to see it fail**

Run: `cd evals && uv run --locked pytest tests/test_tools_arm.py -v`
Expected: FAIL — `KeyError: 'tools'`, then `AttributeError: module 'codecite_evals.task' has no attribute 'codegraph_note'`.

- [ ] **Step 3: Add the arm set**

In `config.py`, after the `"assist"` entry:

```python
    # A third arm for another tool: baseline, codecite and codegraph in one
    # run, so the two tools meet the same host and the same noise.
    "tools": (("baseline", "baseline"), ("codecite", "codecite"), ("codegraph", "codegraph")),
```

- [ ] **Step 4: Lift the corpus words out of `codecite_note`**

In `task.py`, extract the body of `codecite_note` that computes `indexes` into `corpus_words(task_dir, corpus)`, unchanged in behaviour, and have `codecite_note` call it. Then add, below `codecite_note`:

```python
def codegraph_note(task_dir: Path, corpus: str = "") -> str:
    """The codegraph arm's note, mirroring codecite_note: what the corpus
    holds, then the tool's surface. It names no other tool."""
    return (
        f"`codegraph` indexes {corpus_words(task_dir, corpus)} into a symbol and call graph, kept in `.codegraph/`. "
        '`codegraph explore "<words>"` prints the relevant symbols\' source with the call paths between them and '
        "their blast radius in one call, and `codegraph callers|callees|impact <symbol>` answers one question about "
        "a symbol. As an MCP tool, `mcp__codegraph__codegraph_explore` does what `codegraph explore` does."
    )


def note(task_dir: Path, condition: str, corpus: str = "") -> str | None:
    """The arm's note, or None for an arm with no tool of its own."""
    return {
        "codecite": codecite_note,
        "codegraph": codegraph_note,
    }.get(condition, lambda *_: None)(task_dir, corpus)
```

- [ ] **Step 5: Route `prompt()` through `note()`**

Replace the last line of `prompt` with:

```python
    note_text = note(task_dir, condition, corpus)
    return f"{text}\n\n{note_text}\n" if note_text else text
```

- [ ] **Step 6: Run the tests**

Run: `cd evals && uv run --locked pytest tests/test_tools_arm.py tests/test_task.py -v`
Expected: PASS. If `test_the_two_notes_are_of_a_length` fails, shorten or lengthen `codegraph_note` — do not change the codecite note.

- [ ] **Step 7: Commit**

```bash
git add evals/src/codecite_evals/config.py evals/src/codecite_evals/task.py evals/tests/test_tools_arm.py
git commit -m "feat(evals): a tools arm set, and a codegraph arm's note"
```

---

### Task 2: The codegraph arm in the sandbox

**Files:**
- Modify: `evals/inspect/Dockerfile` (after the corpus `RUN`, before `COPY pi-models.json`)
- Modify: `evals/src/codecite_evals/task.py` (`MCP_CONFIG`, `pi_command`, `prepare_arm`, the `pi()` agent)
- Modify: `evals/src/codecite_evals/claude_code.py` (`MCP_CONFIG`, `command`, `MCP`)
- Test: `evals/tests/test_task.py`, `evals/tests/test_claude_code.py`

**Interfaces:**
- Produces: `task.GRAPH = "/opt/codegraph/bin/codegraph"`, `task.PI_MCP = "/root/.pi/agent/mcp.json"`, `task.GRAPH_MCP_CONFIG`
- Produces: `task.CODEGRAPH_TOOLS = ["mcp__codegraph__codegraph_explore"]`
- Produces: `claude_code.GRAPH_MCP = "/root/.claude/codegraph-mcp.json"`, `claude_code.GRAPH_MCP_CONFIG`
- Consumes: Task 1's `note()`; nothing else.

- [ ] **Step 1: Write the failing tests**

In `evals/tests/test_task.py`:

```python
def test_the_codegraph_arm_gets_codegraphs_tools_and_not_codecites():
    cmd = task.pi_command("codegraph", "do the task")
    tools = cmd[cmd.index("--tools") + 1].split(",")
    assert "mcp__codegraph__codegraph_explore" in tools
    assert not any(t.startswith("mcp__codecite__") for t in tools)
    assert "read" in tools


def test_every_arm_but_the_codegraph_one_loses_codegraph():
    for condition in ("baseline", "assist", "codecite"):
        env = asyncio.run(task.prepare_arm(Sandbox(), condition, task.PI_MCP, task.MCP_CONFIG))
        assert env["PATH"].startswith("/usr/local/bin") or condition == "codecite"
        assert "/opt/codegraph/bin" not in env["PATH"]
```

and, with the fake sandbox recording writes/execs, one assertion that `prepare_arm` for `codegraph` strips `/opt/codecite` and keeps `/opt/codegraph/bin` on `PATH`, and one that for `baseline` it runs an `rm -rf` naming both `.codegraph` and `/opt/codegraph`. Follow the existing `Sandbox` fake at `evals/tests/test_task.py:22`; extend it rather than replacing it.

In `evals/tests/test_claude_code.py`:

```python
def test_the_codegraph_arm_gets_its_own_mcp_config():
    cmd = claude_code.command("codegraph", "do the task", "claude-sonnet-5-5", 600)
    assert cmd[cmd.index("--mcp-config") + 1] == claude_code.GRAPH_MCP


def test_only_one_mcp_config_per_arm():
    for condition in ("baseline", "codecite", "codegraph"):
        assert claude_code.command(condition, "p", "m", 600).count("--mcp-config") == (0 if condition == "baseline" else 1)
```

- [ ] **Step 2: Run them to see them fail**

Run: `cd evals && uv run --locked pytest tests/test_task.py tests/test_claude_code.py -v`
Expected: FAIL — no `task.GRAPH`, `pi_command` raises for the unknown condition, `claude_code.GRAPH_MCP` missing.

- [ ] **Step 3: Add the constants and the MCP entries**

In `task.py`, beside `CODECITE`:

```python
GRAPH = "/opt/codegraph/bin/codegraph"
CODEGRAPH_TOOLS = ["mcp__codegraph__codegraph_explore"]
PI_MCP = "/root/.pi/agent/mcp.json"
GRAPH_MCP_CONFIG = {
    "mcpServers": {
        "codegraph": {
            "command": GRAPH,
            "args": ["serve", "--mcp"],
            "exposure": "direct",
        }
    },
    "autoEnableCodemode": False,
}
```

In `claude_code.py`, beside `MCP`:

```python
GRAPH_MCP = "/root/.claude/codegraph-mcp.json"
GRAPH_MCP_CONFIG = {"mcpServers": {"codegraph": {"command": "/opt/codegraph/bin/codegraph", "args": ["serve", "--mcp"]}}}
```

- [ ] **Step 4: Branch `pi_command` on the condition**

`codegraph` takes the same MCP extension as `codecite`, with its own tool list:

```python
    if condition == "codecite":
        flags, tools = [*PI_FLAGS, "-e", "builtin:mcp"], [*PI_TOOLS, *CODECITE_TOOLS]
    elif condition == "codegraph":
        flags, tools = [*PI_FLAGS, "-e", "builtin:mcp"], [*PI_TOOLS, *CODEGRAPH_TOOLS]
```

- [ ] **Step 5: Strip and grant in `prepare_arm`**

`prepare_arm` keeps its signature; the caller passes the arm's config path and config. Replace its body's first branch with:

```python
    if condition in ("codecite", "codegraph"):
        await sb.write_file(mcp_path, json.dumps(mcp_config, indent=2))
    # Every other arm loses both tools, and every `.codegraph/` index, so the
    # arms differ only by the tool under test.
    if condition != "codegraph":
        result = await sb.exec(["sh", "-c", "rm -rf /opt/codegraph /root/.codegraph /work/project/.codegraph /opt/identity-model/.codegraph /opt/corpus/*/.codegraph"])
        if result.returncode != 0:
            raise RuntimeError("codegraph cleanup failed")
    if condition not in ("codecite", "assist"):
        await sb.exec(["rm", "-rf", "/opt/codecite"])
    if condition == "assist":
        for path, text in assist.files().items():
            await sb.write_file(path, text)
    first = {"codecite": "/opt/codecite/bin:", "codegraph": "/opt/codegraph/bin:"}.get(condition, "")
    path = first + "/usr/local/bin:/usr/bin:/bin"
    return {"PATH": path, "HOME": "/root"}
```

Pi 1.2 discovers only `PI_MCP`; the agents select the arm-specific config (and Claude Code its own config path):

```python
        mcp_path, mcp_config = (PI_MCP, GRAPH_MCP_CONFIG) if condition == "codegraph" else (PI_MCP, MCP_CONFIG)
        env = await prepare_arm(sb, condition, mcp_path, mcp_config)
```

and, in `claude_code.command`, `mcp` becomes:

```python
    mcp = {"codecite": ["--mcp-config", MCP], "codegraph": ["--mcp-config", GRAPH_MCP]}.get(condition, [])
```

with `claude()`'s `prepare_arm` call passing `claude_code.GRAPH_MCP, claude_code.GRAPH_MCP_CONFIG` for the codegraph arm.

- [ ] **Step 6: Bake CodeGraph into the image**

In `Dockerfile`, after the corpus `RUN` and before `COPY pi-models.json`:

```dockerfile
# CodeGraph, pinned, with its index built here: a run reads `.codegraph/` and
# never indexes, since the sandbox has no route out but the proxy. Every arm
# but the codegraph one removes both at sample time (task.py).
ARG CODEGRAPH_VERSION=1.6.4
RUN npm install -g --prefix /opt/codegraph @colbymchenry/codegraph@${CODEGRAPH_VERSION} \
 && /opt/codegraph/bin/codegraph telemetry off \
 && for d in /work/project /opt/identity-model /opt/corpus/*; do \
      if [ -d "$d" ] && [ -n "$(ls -A "$d" 2>/dev/null)" ]; then \
        /opt/codegraph/bin/codegraph init -y "$d" >/dev/null || exit 1; \
      fi; \
    done
```

- [ ] **Step 7: Run the tests**

Run: `cd evals && uv run --locked pytest tests/test_task.py tests/test_claude_code.py tests/test_tools_arm.py -v`
Expected: PASS, including the tests already in those files.

- [ ] **Step 8: Commit**

```bash
git add evals/inspect/Dockerfile evals/src/codecite_evals/task.py evals/src/codecite_evals/claude_code.py evals/tests/test_task.py evals/tests/test_claude_code.py
git commit -m "feat(evals): the codegraph arm's sandbox, both agent paths"
```

---

### Task 3: `used` counts the arm's own tool

**Files:**
- Modify: `evals/src/codecite_evals/report.py:85-96` (`NATIVE`, `CLI`, `codecite_calls`)
- Modify: `evals/src/codecite_evals/claude_code.py` (`measures`)
- Test: `evals/tests/test_report.py`, `evals/tests/test_claude_code.py`

**Interfaces:**
- Produces: `report.NATIVE = {"codecite": "mcp__codecite__", "codegraph": "mcp__codegraph__"}`
- Produces: `report.CLI` stays the codecite pattern; add `report.GRAPH_CLI` for `codegraph`, and `report.cli(tool: str) -> re.Pattern`.
- Produces: `report.tool_calls(sample, tool: str) -> int` (renamed from `codecite_calls`; keep `codecite_calls` as a one-line alias so nothing else breaks).
- Produces: `claude_code.measures(stdout: str, tool: str = "codecite") -> dict` — returns `f"{tool}_calls"` in place of `codecite_calls`.

- [ ] **Step 1: Write the failing tests**

In `evals/tests/test_report.py`:

```python
def test_used_counts_the_arms_own_tool():
    """A codegraph run that only called codecite's tools did not use its own."""
    s = sample("codegraph", 1, passed=True)
    assert report.tool_calls(s, "codecite") == 0
    assert report.tool_calls(s, "codegraph") == 0


def test_a_bash_call_running_codegraph_counts_as_using_it():
    s = sample("codegraph", 1, passed=True)
    s.events = [event_with_tool_call("bash", {"command": "codegraph explore 'token'"})]
    assert report.tool_calls(s, "codegraph") == 1
    assert report.tool_calls(s, "codecite") == 0
```

Follow `evals/tests/test_report.py`'s existing helpers for building a sample and a tool call; add the helper beside them rather than restating it.

In `evals/tests/test_claude_code.py`:

```python
def test_measures_count_the_arms_own_tool():
    out = stream(tool_use("mcp__codegraph__codegraph_explore", {}))
    assert claude_code.measures(out, "codegraph")["codegraph_calls"] == 1
    assert claude_code.measures(out, "codecite")["codecite_calls"] == 0
```

- [ ] **Step 2: Run them to see them fail**

Run: `cd evals && uv run --locked pytest tests/test_report.py tests/test_claude_code.py -v`
Expected: FAIL — `tool_calls` takes one argument; `measures` takes one.

- [ ] **Step 3: Make both tool-aware**

In `report.py`, replace the three constants and the function:

```python
NATIVE = {"codecite": "mcp__codecite__", "codegraph": "mcp__codegraph__"}
ATTACHMENT = "attachment://"
# `codecite` or `codegraph` at the start of a shell command: of the line, or
# after `;`, `&`, `|`, `(`, `$(` or a backtick, behind any VAR=value prefixes,
# by bare name or path. `grep codecite` or `ls /opt/codecite` don't run it.
_CLI = re.compile(r"(?:^|[;&|(`\n]|\$\()\s*(?:\w+=\S*\s+)*(?:\S*/)?(?:codecite|codegraph)(?=\s|$)")


def cli(tool: str) -> re.Pattern:
    """The pattern for one tool's binary, so a call to the other's is not counted."""
    return re.compile(_CLI.pattern.replace("(?:codecite|codegraph)", re.escape(tool)))
```

and rename `codecite_calls(sample)` to `tool_calls(sample, tool="codecite")`, replacing `call.function.startswith(NATIVE)` with `call.function.startswith(NATIVE[tool])` and `CLI.search(command)` with `cli(tool).search(command)`. Keep `codecite_calls = tool_calls` as a module alias.

In `claude_code.py`, `measures(stdout, tool="codecite")` uses `NATIVE`-equivalent strings — import nothing from `report` (which imports `config` only); build `f"mcp__{tool}__"` locally, and return `{..., f"{tool}_calls": calls}`.

- [ ] **Step 4: Read the arm's tool in the scorecard**

In `report.table`, the `used` measure becomes arm-relative: derive the arm's tool from its condition (`"codegraph"` → `"codegraph"`, every other condition → `"codecite"`), and read `agent.get(f"{tool}_calls", tool_calls(s, tool)) > 0` — in `scorecard()`, where `Run.used` is set. The `not measured` line stays as it is, but its text names the arm's tool.

- [ ] **Step 5: Run the tests**

Run: `cd evals && uv run --locked pytest tests/test_report.py tests/test_claude_code.py -v`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add evals/src/codecite_evals/report.py evals/src/codecite_evals/claude_code.py evals/tests/test_report.py evals/tests/test_claude_code.py
git commit -m "feat(evals): count each arm's own tool use"
```

---

### Task 4: The scorecard compares every pair

**Files:**
- Modify: `evals/src/codecite_evals/report.py` (`table`, `scorecard`, `per_arm` call sites)
- Test: `evals/tests/test_report.py`

**Interfaces:**
- Consumes: `config.ARMS["tools"]` (Task 1), arm-relative `used` (Task 3).
- Produces: `report.pairs(pair) -> list[tuple[tuple[str, str], tuple[str, str]]]` — every comparison against the reference, then the arms among themselves: for three arms, `[(b, a), (c, a), (c, b)]`.
- Produces: `report.gated(comparison, pair) -> bool` — true only when the compared arm's condition is `codecite` and the reference is `pair[0]`.

- [ ] **Step 1: Write the failing tests**

In `evals/tests/test_report.py`:

```python
def test_three_arms_get_three_comparisons():
    assert report.pairs((("baseline", "baseline"), ("codecite", "codecite"), ("codegraph", "codegraph"))) == [
        (("codecite", "codecite"), ("baseline", "baseline")),
        (("codegraph", "codegraph"), ("baseline", "baseline")),
        (("codegraph", "codegraph"), ("codecite", "codecite")),
    ]


def test_only_codecite_against_the_reference_is_gated():
    pair = (("baseline", "baseline"), ("codecite", "codecite"), ("codegraph", "codegraph"))
    assert report.gated(pair[1], pair) is True
    assert report.gated(pair[2], pair) is False


def test_a_codegraph_loss_never_gates():
    """A codegraph arm that loses reads `reported, not gated`, and the
    codecite comparison beside it still carries its verdict."""
    lines = "\n".join(scorecard_for("tools", codegraph_passes=0, codecite_passes=EPOCHS))
    assert "loses on" not in lines.split("codegraph")[-1] or "reported, not gated" in lines


def test_every_arm_gets_a_row_even_with_no_usable_runs():
    rows = table(scorecard_for("tools", runs={"codegraph": 0}))[1]
    assert [r[1] for r in rows] == ["baseline", "codecite", "codegraph"]
```

Build `scorecard_for` on the existing `log(...)`/`sample(...)` helpers at `evals/tests/test_report.py:55`, passing `arms="tools"`; an existing two-arm test that asserts the current row shape must still pass unchanged.

- [ ] **Step 2: Run them to see them fail**

Run: `cd evals && uv run --locked pytest tests/test_report.py -v`
Expected: FAIL — `report.pairs` and `report.gated` do not exist; `table` raises on a three-arm unpack.

- [ ] **Step 3: Add the two helpers**

```python
def pairs(pair):
    """Every comparison this arm set makes: each arm against the reference,
    then the arms among themselves, in arm order."""
    return [(arm, pair[0]) for arm in pair[1:]] + [
        (b, a) for i, a in enumerate(pair[1:]) for b in pair[i + 2 :]
    ]


def gated(comparison, pair):
    """Whether a comparison carries the gate: only the codecite arm against
    the reference does. Every other comparison is reported."""
    code, base = comparison
    return code[1] == "codecite" and base == pair[0]
```

- [ ] **Step 4: Render one row per arm and one interval row per comparison**

In `table`, replace the two-arm unpack with the reference arm plus a loop over `pairs(pair)`:

- the run table gets one row per arm in `pair` order, the reference arm's verdict cell empty;
- each comparison's verdict goes in the row of its *compared* arm; low tool adoption reads `not measured` for gated and reported comparisons, and the CodeGraph-vs-codecite comparison checks both arms; only the existing gate uses `losses(found)`, while other comparisons are `reported, not gated`;
- the A/A branch becomes `len({c for _, c in pair}) == 1`, so a three-arm set is never A/A;
- the intervals table gets one row per comparison, in `pairs()` order, each labelled `<code> vs <base>`;
- `aa`'s `pooled_rate` loop still runs over every arm's runs.

- [ ] **Step 5: Run the tests**

Run: `cd evals && uv run --locked pytest tests/test_report.py -v`
Expected: PASS, including every test that was there before.

- [ ] **Step 6: Commit**

```bash
git add evals/src/codecite_evals/report.py evals/tests/test_report.py
git commit -m "feat(evals): a scorecard over any number of arms"
```

---

### Task 5: Docs, and the run that answers the question

**Files:**
- Modify: `evals/README.md` (the arms paragraph)
- Modify: `docs/design.md` (Evaluation plan, the **Arms** bullet)
- Test: `evals/tests/test_config.py` (no new test; the suite is the check)

**Interfaces:**
- Consumes: everything above. Produces nothing.

- [ ] **Step 1: Document the arm**

In `evals/README.md`, where the arms are listed, add one paragraph: `-T arms=tools` runs baseline, codecite and codegraph in one run, so the two tools meet the same host; the codegraph arm gets a mirrored note, CodeGraph 1.6.4 baked into the image with its index built at build time, and every other arm strips it; codegraph comparisons are reported, never gated, and `used` counts the arm's own tool. Name the design doc.

In `docs/design.md`'s **Arms** bullet (Evaluation plan), add the same in one sentence, and one more saying what it settles: whether codecite earns its place beside a tree-sitter graph tool, before milestone 8's adapter is built.

- [ ] **Step 2: Run the whole suite**

Run: `cd evals && uv run --locked pytest`
Expected: PASS, every test.

- [ ] **Step 3: Prove the frozen tasks are untouched**

Run: `git diff --stat main -- evals/tasks/`
Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add evals/README.md docs/design.md
git commit -m "docs(evals): the tools arm, and what it measures"
```

- [ ] **Step 5: Record the run commands, and stop**

The smoke run (needs Docker and the Claude token; not part of CI):

```bash
cd evals && inspect/prepare.sh fastapi-middleware
uv run --locked inspect eval codecite_evals/fastapi_middleware -T arms=tools --model claude-code/claude-sonnet-5-5 --epochs 2 --log-dir inspect/logs/smoke-tools/fastapi-middleware
uv run --locked codecite-eval-report inspect/logs/smoke-tools/fastapi-middleware
```

then the measurement, still in `evals/`: `inspect/prepare.sh backchannel-logout` followed by `TASK_ID=backchannel-logout ARMS=tools inspect/panel.sh quick`. Report both scorecards on #275. The owner runs them and merges; this plan does not.

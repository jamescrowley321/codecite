# A codegraph arm for the agent eval

## Intent

codecite's graph tools are thin next to CodeGraph's: `INSERT INTO edge` runs in
one place (`crates/codecite-core/src/store/graph.rs:786`, the SCIP path), so
`refs` and `graph` answer nothing for a corpus with no SCIP index, while
CodeGraph ships extracted call and import edges for 30+ languages from its own
tree-sitter pass. The design already names CodeGraph a provider to import
(`docs/design.md:363`, milestone 8) and forbids linking its code
(`docs/design.md:367`).

Before building that adapter, measure what it would buy. The eval can answer
whether an agent given CodeGraph does better than an agent given codecite on the
same task, and the answer decides the adapter's shape and whether it is worth
building at all. Nothing in the harness measures CodeGraph today.

The decision this settles: does codecite earn its place beside CodeGraph, and
where does CodeGraph alone beat it.

## Goals

- One run in which the same agent, model, task and host produce a baseline arm,
  a codecite arm and a codegraph arm, so the three are comparable without
  confounding one run's host noise with another's.
- The existing gate is unchanged: codecite against baseline still decides
  whether a codecite PR blocks.
- Each arm's own tool use is counted, so an arm that ignores its tool reads
  `not measured` rather than `no loss`, as a codecite arm does today.

## Non-goals

- No CodeGraph adapter, import or dependency in `codecite` itself.
- No change to any frozen task: `task.json`, `prompt.md`, `project/`, `hidden/`
  and `checks.py` under `evals/tasks/<id>/` stay as they are.
- No new gate. A codegraph arm's loss blocks nothing.

## Anti-goals

- An arm whose advantage could come from its prompt note rather than its tool.
- A scorecard that reports an arm's runs as comparable when its sandbox differed
  by more than the tool (a leftover `.codegraph/` in the baseline arm, say).

## Constraints

- Two agent paths, not one: pi (`task.py`) and Claude Code (`claude_code.py`,
  the gating panel).
- The sandbox has no route out but the proxy that lets through Claude Code's API
  host, so CodeGraph must index at image build and answer offline.
- The image is `node:24-trixie-slim` with node already installed; a `linux-x64`
  prebuilt is what CodeGraph publishes (npm `@colbymchenry/codegraph@1.6.4`,
  MIT).

## Approaches

1. **A second pair, `arms=graph`** (baseline vs codegraph). About 40 lines:
   `config.ARMS`, `prepare_arm`, `pi_command`, `claude_code.command`, `prompt`,
   and a "reported, not gated" marker in `report.py`. Everything else works
   untouched. codecite against codegraph then compares two separate runs, each
   with its own host and noise, which is what the A/A arm exists to measure.
2. **Three arms in one run, `arms=tools`** (baseline, codecite, codegraph).
   `report.py` learns N arms and prints an interval row per comparison against
   the reference. About 150 lines, mostly `table()`. One run, one host, one
   noise regime: the only shape in which the two tools are directly comparable.
3. **No arm**: drive both CLIs by hand over the history benchmark's mined
   queries. Cheap, but it measures retrieval only, never turns or wall time,
   which is what #93 says is broken.

**Chosen: 2**, and the first measurement runs on `backchannel-logout`, the task
built to make finding the right code the expensive part (`docs/design.md:590`),
which is CodeGraph's whole claim.

## Decisions

- **The arm set is `tools`.** `config.ARMS["tools"]` is
  `(("baseline", "baseline"), ("codecite", "codecite"), ("codegraph", "codegraph"))`.
  Sample ids stay the arm label, so `per_arm` and the scorer need no change.
- **The codegraph arm gets a mirrored note.** `codegraph_note()` has the same
  shape, length and job as `codecite_note()`: the agent snippet naming what the
  corpus indexes, then one line on the tool's surface. Neither note names the
  other tool. Without this, a win is attributable to note length rather than to
  the tool.
- **CodeGraph is pinned and baked in.** `@colbymchenry/codegraph@1.6.4` is
  installed into the image at build time, `codegraph telemetry off`, then
  `codegraph init -y` over `/work/project` and `/opt/identity-model` (or the
  task's corpus sources), leaving a `.codegraph/` at each. Measured on
  identity-model: 567 files, 11,224 nodes and 31,600 edges in 656 ms, a 33 MB
  `.codegraph/`, and one `codegraph explore` call returning 25 KB. A run needs
  no network: indexing is tree-sitter, and every MCP tool has a CLI twin.
- **The baseline arm strips it.** `prepare_arm` already removes `/opt/codecite`
  for the baseline arm; it removes `/opt/codegraph` and every `.codegraph/` too,
  so the arms differ only by the tool.
- **MCP is written by hand, not by `codegraph install`.** That command knows
  Claude Code, Codex and nine others but not pi, and the harness already writes
  both configs itself. The entry is `{"command": "/opt/codegraph/bin/codegraph",
  "args": ["serve", "--mcp"]}` for both agents.
- **Reported, never gated.** `report.py` gains a set of arm names that are
  reported, so a `tools` pair's codegraph comparisons read `reported, not
  gated`, the way a model outside the gating panel does (`UNGATED`). The
  codecite-vs-baseline comparison keeps its gate exactly.
- **Three interval rows.** `table()` prints codegraph-vs-baseline,
  codecite-vs-baseline and codegraph-vs-codecite, each through the existing
  `interval()` with the same measures and the same margin. A ratio of ratios
  would not be the interval the bootstrap measures; a direct comparison is.
- **`used` is arm-relative.** It is a call to the arm's own tool: an MCP tool
  named for that server, or a shell command running that binary, as
  `codecite_calls` counts today. A codegraph arm with no such call reads
  `not measured`.
- **Smoke before panel.** `fastapi-middleware` at one model and two epochs
  proves the image, the two agent paths and the scorecard; then
  `backchannel-logout` on the gating panel is the measurement.

## Left to the builder

- Where the note lives and how `prompt()` chooses it: one `note()` that takes the
  condition, or a per-arm function.
- How `report.py` expresses "reported": a set of arms names, or a flag on the
  arm tuple.
- Whether the codegraph arm also gets the CLI on `PATH`, as the codecite arm
  does. Recommended: yes, mirroring codecite exactly, since an agent searches
  through the shell as often as through its tools.
- The image's codegraph install line, and whether `init` runs over one directory
  or several (a size-dial corpus has sources under `/opt/corpus`).
- Which task runs after `backchannel-logout`, if the first result is ambiguous.

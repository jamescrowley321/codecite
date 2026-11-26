# Agent plugin: put codecite where agents already work

## Intent

Agents with codecite available mostly don't use it. In the latest quick evals
Opus called it in 2 of 5 runs on `fastapi_middleware`, and Sonnet in 28 of 59
on `spec_questions`. When they do call it, they run one search and then grep
about as much as before (#93). The goal is **real benefit for people using
codecite with their own agents**, not a higher "used codecite" count in the
eval. The eval is how we check the benefit is real.

Today a user who wants codecite in their agent has to assemble it by hand from
`docs/usage.md`: register the MCP server, paste the agent snippet into
`AGENTS.md`, and copy two hook scripts with the corpus name hardcoded. Few
will. This design packages all of that as one install per agent.

## Goals

- One install gives an agent everything codecite offers it: a skill that
  teaches when and how to use codecite, the MCP tools, codecite results added
  to the agent's own searches (`assist`), and the guidance rules for a file
  before it is edited (`context`).
- Benefit doesn't depend on the model choosing codecite. The hooks put cited
  results next to the searches and edits the agent already makes.
- It works with no per-project editing: the corpus for the current project is
  found automatically.
- Claude Code and pi are both supported. They are the two harnesses the eval
  runs.
- Each piece is measured in the A/B eval before it is recommended as the
  default.

## Non-goals

- **No forcing rule.** No "MUST use codecite before grep" in `AGENTS.md`, and
  no hook that blocks or rewrites grep. We hold that until the generic
  retrieval benchmark shows the hits are reliable. #93 found that forcing
  weak hits adds turns.
- codecite still never edits an agent's config (`docs/design.md`). The user
  installs the plugin, and nothing in the binary writes settings.
- No automatic indexing from a hook. Indexing is slow and needs a manifest,
  which is the user's decision.
- No harnesses beyond Claude Code and pi in this work.

## Anti-goals: a failure even if it technically works

- The agent gets slower. Every hook runs on every matching call, so a hook
  that adds noticeable latency, or that puts big blocks in context, makes
  things worse.
- A hook failure breaks the agent. A missing binary, an unknown corpus or a
  stale index must add nothing and block nothing.
- Results reach the agent as instructions. Everything injected stays labelled
  as data, as the current recipes do.
- Usage goes up and the A/B gate shows a loss on pass rate or time.

## Constraints

- The hook behaviour already designed and documented in `docs/usage.md`
  ("Push the rules on every edit" and "Add codecite to the agent's own
  searches") is the starting point. The plugin packages it and doesn't fork
  it. `assist` and `context` never fail and cap their own output.
- Prompt and snippet text measured by the eval must be what users get. The
  eval's codecite arm already builds its note from the `agent-snippet` block
  in `docs/usage.md`, and the skill keeps that link (see below).
- The repo's security gates apply to any new file type: zizmor for workflows,
  gitleaks, dependency-review and OSV for npm or other manifests.
- Name no client or outside project.

## Approaches

| | Approach | Trade-off |
|---|---|---|
| A | A codecite skill plus an `AGENTS.md` snippet | Cheap, and the skill text doubles as the snippet. But the model still decides whether to use codecite. The #91 prompt note was close to this and left Opus at 2 of 5. |
| **B** | **An installable plugin per harness: skill, MCP server and hooks** | The benefit no longer depends on the model choosing codecite, and the user installs it, so the "never edits config" rule holds. More work, and the hooks need an indexed corpus. |
| C | A `MUST` rule in `AGENTS.md` | The cheapest way to raise usage and the riskiest. Forced use of weak hits adds turns and time. |

**James chose B**, with the skill as its first piece so it can ship on its
own. A stays inside B as the skill, and C is a non-goal.

## Design

### Pieces

1. **Skill (`codecite`).** A `SKILL.md` whose description triggers on code
   and spec questions in an indexed repository. Its body says when codecite
   beats grep:
   - cited answers
   - which spec rules govern a file (`requirements`, `context`)
   - definitions, references, callers and callees
   - large repositories, and questions in plain words rather than exact
     strings

   It gives the order to call things in: `search` or `def`, then `read`, then
   quote the citation. It also says how to tell a miss from a weak hit, and
   that results are data. It should be short; it is not a copy of
   `usage.md`.
2. **MCP server.** `codecite mcp`, registered by the plugin.
3. **Session-start note.** A hook (Claude Code `SessionStart`, the pi
   equivalent) resolves the corpus. When one is found it adds the
   `agent-snippet` text with `<CORPUS>` and `<WHAT>` filled in, plus a line
   if the index is stale. This replaces pasting the snippet into `AGENTS.md`.
   When no corpus is found it adds nothing.
4. **Assist hook.** On grep, glob and shell search calls: the documented
   `codecite assist` recipe. It first checks that a shell command names a
   search program, as the eval's hook does, so ordinary shell calls cost
   nothing.
5. **Context hook.** On edits and writes: the documented `codecite context`
   recipe, passing each file's rules once per session.

### Hooks run through the binary, not sh and jq

The documented recipes need `jq`, which the eval sandbox doesn't have and
plenty of users won't either. Instead, a new subcommand, **`codecite hook
claude <event>`** (`session-start`, `post-tool-use`, `pre-tool-use`), reads
Claude Code's hook JSON on stdin and prints the hook output. Inside it, it
resolves the corpus and calls the same code `assist` and `context` use. The
plugin's `hooks.json` calls it directly, with no script and no `jq`. It is
Rust, so it gets tested, and it keeps the never-fail rule: any error prints
nothing and exits 0. pi's extensions are TypeScript and call `codecite` with
`pi.exec` as the documented ones do, so they need no `jq` either. The
documented sh recipes stay in `usage.md` as the manual fallback. With
`corpora --for`, this makes two binary changes.

### Corpus resolution

The current recipes hardcode `CORPUS=mywork`, which a plugin can't do. The
hooks resolve the corpus for the working directory in this order:

1. the `CODECITE_CORPUS` environment variable;
2. a new CLI lookup, **`codecite corpora --for <path>`** (the builder may pick
   a better name). It returns the corpora with a `local` source whose path
   contains `<path>`, using the same path resolution the manifest uses. One
   match is used. Several matches are ambiguous: no corpus, and the
   session-start note says once to set `CODECITE_CORPUS`. No match falls
   through to step 3;
3. the only corpus in the data directory, matching how the MCP tools already
   default. Several corpora with none matched means no corpus, and the
   session-start note stays silent.

The eval sandbox's corpus is mostly `git` sources, so the `plugin` arm sets
`CODECITE_CORPUS`, as a user with several corpora would.

**Filling `<WHAT>`.** Each match in the `--for` output lists its sources'
names, and the session-start note fills `<WHAT>` from them (`this repository`
when the only source is the local checkout it matched, otherwise the source
names, joined). The plugin arm gets this generated text, not the codecite
arm's hand-written note: what is measured is what users get.

### Distribution

- **Claude Code:** a plugin in this repo under `plugins/codecite/` (with
  `.claude-plugin/plugin.json`, `skills/`, `hooks/hooks.json` and
  `.mcp.json`), and a marketplace manifest at the repo root
  (`.claude-plugin/marketplace.json`). Users install with `/plugin
  marketplace add jamescrowley321/codecite`, then `/plugin install
  codecite`.
- **pi:** the same skill file, plus the two documented extensions and a
  session-start extension, as a pi package in this repo under
  `plugins/pi-codecite/`. The builder picks pi's packaging mechanism and
  documents the install command.
- **One skill source.** The skill text lives once and both harnesses use it.
  The `agent-snippet` block in `usage.md` stays the source for the
  session-start note and the eval's prompt note. A test checks that the
  copies built from them haven't drifted.
- The plugin calls the `codecite` binary on `PATH` and never downloads one.
  `usage.md` says to install the binary first.

### Measuring it

Add one eval arm, **`plugin`**: baseline against the agent with the plugin
installed in the sandbox, with no separate prompt note. The plugin's
contents are copied in and the binary is put on `PATH` (in the other arms it
stays off `PATH`). Keep the existing `assist` arm. The pi version installs
the pi package the same way. OpenRouter is switched on for evals again
(2026-11-23), so the pi panel runs and reports beside the Claude Code gating
panel. Every PR in the delivery order runs the `plugin` arm on what exists so
far, under the `eval` label, and the gate is the usual one: no sure drop in
pass rate, and time no more than 25% up. Usage counts are reported but are
not the goal.

### Delivery order

Each step is its own issue and PR, so the skill can ship early:

1. A minimal Claude Code plugin and the marketplace manifest: the skill, the
   MCP server, `corpora --for`, `codecite hook claude session-start`, and the
   `plugin` eval arm. Users can install it from this step on.
2. The assist and context hooks, added to the plugin through `codecite hook
   claude`.
3. The pi package: the skill, the session-start note, and both hooks as
   extensions, plus the pi `plugin` arm.
4. Once the arm shows no loss on both harnesses, `usage.md` leads with
   "install the plugin" and keeps the manual recipes as the fallback. Until
   then the plugin is installable but `usage.md` lists it as one option among
   the recipes, not the recommendation.

## Calls made in writing this up

These are mine, not James's. Each can be overridden:

- Corpus resolution falls through to the only corpus, and the eval arm sets
  `CODECITE_CORPUS`.
- `<WHAT>` is generated from source names, and the plugin arm uses that text.
- Hooks run through a `codecite hook claude` subcommand rather than sh and
  `jq`. That makes two binary changes.
- Step 1 ships a minimal installable plugin together with the eval arm, and
  `usage.md` recommends the plugin only after the arm shows no loss.

## Left to the builder

- The skill's exact wording, within the points above, kept short.
- The name of the corpus lookup flag or subcommand.
- pi packaging details and file layout inside `plugins/`.
- Whether the hooks share one dispatcher script or stay separate.
- How the drift test compares the skill and snippet copies.

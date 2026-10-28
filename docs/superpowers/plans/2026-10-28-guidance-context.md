# Guidance sources and `codecite context` (#161)

Index a team's rules as a guidance source, and hand an agent the rules that
govern a file before it edits it.

## Decisions

- **Stored as a document.** A guidance source is a `document` source with
  `guidance = 1`, so `search`, `grep`, `read` and `cite` treat its rules as
  spec sections with no change. Scopes live in a new `rule_scope` table keyed
  by source and rule number (migration 0005), rewritten on every index run
  because the manifest can change them without the bytes changing.
- **Two origins.** `path` (a local Markdown file, hashed at every run, so an
  edit is a new version) or `url` + `sha256` (fetched and pinned through the
  document fetcher).
- **Rules from headings.** Numbered headings are rules as written when any
  heading has a number; otherwise the outline numbers them, skipping a lone
  title. Only sections holding a BCP 14 keyword come back from `context`.
- **Scope beside the rule.** A `<!-- codecite-scope ... -->` comment holds
  TOML (`paths`, `languages`, `imports`): invisible when the Markdown
  renders. Precedence: manifest `rules."<n>"`, the comment, the parent rule,
  the manifest `scope`.
- **Globs as `include` globs:** `globset` with `*` inside one directory.
- **Language and imports in core.** `Language` moves from `codecite-index`
  to `codecite-core` (index re-exports it), and a line-scan import reader
  joins it, so the MCP server, which depends on core only, can describe a
  file.
- **Budget.** Each rule's text is a snippet (40 lines, 4 KiB); at most 50
  rules and 64 KiB of text, the rest cited in `omitted`.
- **Hooks are recipes.** A Claude Code PreToolUse hook (`additionalContext`)
  and a pi extension (appends to the first `edit`/`write` result per file)
  are documented in `docs/usage.md`; codecite never edits agent config.

## Steps

1. Core: move `Language`; add `imports()` with tests per language.
2. Core: migration 0005; `RuleScope`, `ContextTarget`, `Store::context`,
   `Store::set_guidance`, with tests for glob, language and import scopes, a
   rule with no scope, ordering, the byte budget and staleness.
3. Index: `markdown_headings` from the Markdown chunker; `split_guidance`
   with tests for both numberings, scope comments and errors.
4. Index: manifest `kind = "guidance"` with origin, hash, rule-number and
   glob checks; `index_guidance` with scope precedence; index tests that
   read a rule back and ask `context`.
5. CLI `codecite context` (`--root`, `--stdin`, `--json`) and MCP `context`
   tool; end-to-end tests, and the MCP-equals-CLI byte check.
6. Docs: `docs/usage.md` (guidance, `context`, both hooks) and
   `docs/design.md` (data model, pipeline, manifests, interfaces).

## Measuring it

The push arm runs against #160's `house-rules` task, which isn't built yet.
Its quick-tier scorecard follows once #160 lands.

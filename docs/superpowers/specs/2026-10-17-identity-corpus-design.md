# The identity corpus: dogfooding codecite on identity-model

## Intent

codecite's first real corpus is the owner's own auth work: the
`identity-model` monorepo (`~/repos/auth/identity-model`, py, go, rust and node
ports plus shared conformance vectors) and the OAuth and OpenID specs it
implements. The point is twofold: better auth code while identity-model is being
built, and testing codecite on real work every day instead of on benchmarks only.

The problem it has to solve is already documented. identity-model's epic #852
audited about 365 OIDC citations and found 13 kinds of error spread over about 80 lines.
Every cited section existed. The errors were the wrong step or sub-section
(auth_time cited as Core §3.1.3.7 step 12 instead of 13), MUST written where the spec
says SHOULD, and text from superseded errata. Five of them became wrong behavior,
such as azp enforced unconditionally and a Back-Channel Logout token without `exp`
accepted. These are the errors an agent makes when it cites a spec from memory.
The issue tracker also shows docs claiming behavior the code doesn't have
(#645, #730) and drift between the language ports (#573).

## Goals

1. **Spec text by address.** An agent asks for `OIDC Core §3.1.3.7 step 13` or
   `RFC 9449 §4.3` and gets the exact text of that section or step, its
   requirement strength (MUST, SHOULD, MAY and their negations), and a citation
   pinned to the exact document version.
2. **Citation checking as an agent tool.** Given a citation and, optionally, the
   claim written beside it, codecite says whether the section and step exist, returns
   the governing text with its strength, and flags a strength mismatch (claim says
   MUST, text says SHOULD) or a superseded document. The agent judges the meaning;
   codecite supplies the facts.
3. **Specs pinned without git.** RFCs and OpenID specs are fetched documents,
   pinned by URL and content hash, and marked when superseded (RFC 7234 is
   obsoleted by RFC 9111; Core 1.0 is current only with errata set 2).
4. **identity-model as a code source.** Its working tree is indexed alongside the
   specs, so one query covers how the ports implement a rule and what the spec says.
5. **In use early.** Claude Code working in identity-model has these tools over
   MCP by the end of M3, before the evaluation and graph milestones. MCP is set in
   the owner's user-level Claude Code config, so nothing changes in the public
   identity-model repo.

## Non-goals

- A CI gate in this round. identity-model#861 keeps its own vendored index for
  now. Running codecite's citation checking in identity-model's CI is a later
  issue (see Milestones). The design keeps that path open: citation parsing and
  checking live in `codecite-core`, not in the MCP layer.
- Judging meaning with a model. codecite stays local and deterministic. It does no
  network access at query time and makes no LLM calls.
- Fixing identity-model's citations. That's identity-model#853 and its sibling
  issues, done in that repo with codecite's help.
- General web or document search beyond the pinned spec list.

## Anti-goals (failure even if it works)

- codecite returning spec text without its pinned version, or paraphrasing it.
  A wrong citation from codecite is worse than none.
- A checker that confidently passes a wrong step citation. When codecite cannot
  resolve a citation it says so with candidates, never a silent pass.
- Re-fetching specs on every run, or results changing because a website changed.
  Content changes are refused until the pin is updated on purpose.

## The corpus

One corpus named `identity`, defined by a manifest committed at
`corpora/identity.toml` in this repo.

**Code source:** the identity-model primary checkout, `~/repos/auth/identity-model`,
read in place (`kind = "local"`). It is the owner's decision to index only this
checkout, not the per-branch worktrees: one fixed index of `main`, refreshed by
re-running `codecite index identity` after a pull, which is incremental.
`.gitignore` is respected, so `target/`, `.venv/`, `node_modules/` and build output
are skipped. Results cite `identity-model@<HEAD sha>:<path>#L<a>-L<b>` and carry
`dirty: true` when the cited file has uncommitted changes. Its license is Apache-2.0
and its class `adapt`.

**Spec sources** (`kind = "document"`), fetched as HTML:

- **OpenID**, from `openid.net/specs/`:
  - Connect Core 1.0 incorporating errata set 2
  - Discovery 1.0 errata 2
  - Dynamic Client Registration 1.0 errata 2
  - RP-Initiated Logout 1.0
  - Back-Channel Logout 1.0 errata 1
  - FAPI 2.0 Security Profile
  - FAPI 2.0 Message Signing
- **RFCs**, from `www.rfc-editor.org/rfc/rfcNNNN.html`: 6749, 6750, 7009, 7515,
  7517, 7518, 7519, 7521, 7523, 7591, 7592, 7636, 7638, 7662, 8414, 8628, 8693,
  8705, 8707, 8725, 9101, 9126, 9207, 9396, 9449 and 9700, plus 2119 and 8174,
  which define the requirement keywords.

This list is every spec identity-model cites more than a handful of times
(counted across the repo on 2026-10-17). Adding one later is a manifest edit.

A spec is pinned by `url` and `sha256` of the fetched bytes. If a fetch returns
different bytes, indexing stops with an error naming the document; the owner
re-pins on purpose. Spec text license: IETF Trust for RFCs, OpenID Foundation for
OpenID specs. Both are quoted with attribution, and neither is redistributed,
because the manifest holds URLs and hashes only. Spec results carry
`license_class = "quote"`, a new class meaning "quote exactly with the citation;
this is normative text, not code to adapt".

## Spec structure

Each spec document is split into **sections** (one chunk per numbered section,
nested sections kept as their own chunks with their parent recorded) and
**steps** (each item of a numbered list inside a section, e.g. the 13 validation
steps of Core §3.1.3.7, addressable on its own).

Every section and step stores:

- its number (`3.1.3.7`), title, and the document's own anchor
  (`#IDTokenValidation`, `#section-4.3`), so a citation can be given either way and
  a link's anchor can be checked against its § label;
- its requirement keywords as written: MUST, MUST NOT, REQUIRED, SHALL,
  SHALL NOT, SHOULD, SHOULD NOT, RECOMMENDED, NOT RECOMMENDED, MAY, OPTIONAL;
  uppercase only, per RFC 8174;
- the document's version: pinned hash, title with errata level, and for RFCs the
  "Obsoletes", "Obsoleted by" and "Updated by" relations from the RFC header.

Spec citations look like
`rfc9449@sha256:1a2b3c4d5e6f#section-4.3` and
`openid-connect-core-1_0@sha256:…#section-3.1.3.7/step-13`. `read` accepts these and also
the forms people write: `RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`,
`Core 3.1.3.7(13)`, `https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation`.
A bare `§` without a document name is ambiguous and returns candidates.
identity-model#861 found that guessing a document produced false positives.

## Citation checking

A new operation, `cite`, in the CLI and MCP:

- **Input:** `citation` (any accepted form), optional `claim` (the text written
  beside it), optional `corpus`.
- **Output:** the result shape plus
  - `resolved`: true or false;
  - `candidates`: when not resolved;
  - `text`: the exact section or step text, capped like snippets;
  - `strength`: keywords found;
  - `superseded_by`: when the document is obsoleted, or a newer errata exists in
    the corpus;
  - `findings`: a list of typed mismatches.
- **Mismatch types for this round:**
  - `no_such_section`, `no_such_step`;
  - `anchor_label_mismatch`: the link's anchor points elsewhere than its § label;
  - `strength_mismatch`: `claim` contains an uppercase keyword stronger or weaker
    than the text's;
  - `superseded_document`: the cited document is obsoleted, or the claim
    matches only an older pinned version's text (see Versions below);
  - `topic_mismatch`: `claim` names a protocol term (`auth_time`, `azp`, `nonce`,
    `at_hash`, …) that does not appear in the cited step or section but does
    appear in exactly one sibling step or section. The finding names that
    sibling as the suggested citation. This is the 12-versus-13 error. When the
    term appears in several siblings, or in none, there is no finding, only
    `suggested` candidates;
  - `ambiguous_document`.
- **Bare §.** A citation with no document name (`§3.1.3.7 step 12`) resolves when
  exactly one current document in the corpus has that section and step. Otherwise
  the result is `ambiguous_document` with candidates. This generalizes
  identity-model#861's rule, where a bare § counts as Core only for §3.1.3.7, which
  is how that prototype reached 0 false positives.
- How the term vocabulary and term matching work is the builder's choice, within
  the rule above.

## Versions

Out-of-date citations are one of the errors this exists to catch, so a document id
can have several pinned versions:

- **Current version.** One per document id, marked `current = true` in the
  manifest. `read` and `cite` resolve to it unless a citation names a version.
- **Older versions.** Pinned for the documents where identity-model has cited old
  text: Core 1.0 as first published (2014), Back-Channel Logout before errata 1,
  and Connect Basic Client Profile draft 32. When a `claim` matches only an older
  version's text, the finding is `superseded_document`, and it gives the current
  text.
- **Relations between documents.** For example, RFC 7234 is obsoleted by RFC 9111.
  These are written in the manifest by hand (`obsoleted_by`, `updated_by`) and
  never fetched, so they stay pinned like everything else. RFC 8650 and later
  are published as v3 HTML, and the published file is not changed afterwards, so
  it does not show what later obsoleted it.

## Changes to milestones

The existing order stays; scope moves earlier so the identity corpus works at M3.

- **M2 Index and search** gains:
  - document sources pinned by URL and hash;
  - spec section and step chunking with anchors and requirement keywords;
  - the minimal manifest (`kind = "local" | "git" | "document"`, `url`/`path`,
    `commit` or `sha256`, `license`, `use`);
  - `corpora/identity.toml`;
  - `read` of spec citations.

  The acceptance target is the identity corpus: it indexes cleanly and a fixed list
  of lookups returns the right sections. SPDX detection, `add`, and license-class
  enforcement stay in M5.
- **M3 MCP server** gains `cite` with the findings above and is wired into Claude
  Code for identity-model work through the owner's user-level config.
- **M4 Evaluation** adds a citation benchmark from identity-model's own audit:
  - **Extractor.** M4 adds a citation extractor. It finds every citation in a
    tree, including ones split across two lines, and pairs each with its
    surrounding lines as the claim. This is the scanning half of the later
    `cite check`; M4 needs it to run the benchmark.
  - **Labelled set.** The #861 prototype's output is lost, so the set is rebuilt
    by hand. It comes from the audit (`docs/oidc-citation-audit-2026-10-03/00-audit.md`
    and C1–C7 in identity-stack-planning), re-checked against identity-model at
    `1e3b6a6`, whose audit line numbers are off by one in places. The owner
    approves the list before it is used.
  - **The set contains:**
    - the 48 section and step violations: 44 step-topic errors, 3 Back-Channel
      §2.4 step lines and 1 anchor/label mismatch;
    - the strength errors the audit found;
    - every other citation at that commit, labelled correct.
  - **Measure:**
    - `cite` flags all 48;
    - 0 false positives on the correct citations;
    - how many strength errors `strength_mismatch` catches.
- **Later (M5 or a new issue):** `codecite cite check <paths>`, which runs the M4
  extractor and `cite` on each citation and exits non-zero on findings, for use in
  identity-model CI in place of `spec/oidc-sections/*.tsv`.

## Success

- In daily identity-model work, agents get spec text from codecite instead of
  memory, and every spec statement in a PR carries a pinned citation.
- On the audit benchmark, `cite` catches all 48 known section and step errors with
  no false positives, and reports strength for every resolved citation.
- The bugs found during dogfooding become codecite issues. Losses get reported
  alongside wins, as the evaluation plan already requires.

## Left to the builder

- HTML parsing approach for the three formats:
  - xml2rfc v3 RFC HTML (RFC 8650 and later);
  - older RFC HTML;
  - the xml2rfc-generated OpenID pages.

  Tests must cover a real document of each format.
- How steps are detected inside a section (numbered `<ol>` items, or numbered
  paragraphs in older formats). Core §3.1.3.7, Back-Channel §2.6 and RFC 9449 §4.3
  must come out with the right step counts.
- The topic-hint method, within the limits above.
- Where `corpora/` manifests and the fetched-document cache live on disk, as long
  as fetched documents are never committed.

## Decided by Claude, not the owner (check these)

- The spec list above, taken from citation counts in identity-model.
- `license_class = "quote"` for spec text.
- Pinning documents by `sha256` and refusing changed content.
- Citation format `<doc>@sha256:<12 hex>#section-<n>[/step-<k>]`.
- Pulling the minimal manifest into M2 while license detection stays in M5.
- `topic_mismatch` as a finding when exactly one sibling holds the term; otherwise
  only suggestions.
- Bare § resolves when exactly one current document matches.
- Pinning older versions (Core 2014, Back-Channel before errata 1, Basic draft 32)
  and keeping document relations hand-written in the manifest.
- The citation extractor moving into M4.

# Spec version diff — design

Issue #158. A corpus can pin several versions of one document
(`openid-connect-backchannel-1_0` and its pre-errata `-final`), but nothing
tells an agent that they differ, or how. Errata set 1 of Back-Channel Logout
1.0 made `exp` REQUIRED in §2.4; the version before it has no `exp` at all,
and models trained on both follow the older one.

This spec settles the open question: how to pair a section across versions
and how to tell a normative change from rewording. No one was available to
brainstorm it, so it decides and gives reasons, and it keeps to what the
index already stores.

## What is compared

Everything comes from the chunks the index already has: each version's
`section` and `step` chunks, with their text, numbers, anchors and line
ranges. No schema change and no re-index.

- **Versions.** `to` is the current version unless asked for; `from` is the
  older pinned version the corpus stored last (the highest `source.id` that
  is not current), unless asked for. The index records no publication dates,
  so "latest older" means the one pinned last; with one older version, the
  usual case, there is no choice. Either can be named by its hash, with or
  without `sha256:`, as in a citation.
- **Sections** pair by number, then by anchor for a section renumbered
  between versions. A section only one version has contributes its
  statements as all added or all removed.
- **Statements.** A section's text, its heading line dropped, splits into
  units: each step is one unit, cited by its step citation; every other line
  is one unit, cited by its section. Two layout facts shape this. An OpenID or
  v3 RFC page renders a paragraph, list item or definition per line, and a
  definition's term (`exp`) on its own line before its description
  (`REQUIRED. Expiration time, …`); a term line (at most four words, no
  sentence punctuation, followed by a non-blank line) joins the line after it,
  with `\n` between, so the unit is still exact text. An older RFC's plain
  text is wrapped `<pre>`, so a run of lines that all start with whitespace,
  up to a blank line, is one unit.
- **Normative** means the unit holds a BCP 14 keyword (`requirement_keywords`,
  the same scan `read` and `cite` use) or is a step: a numbered step sits
  under a sentence such as "the RP MUST validate the Logout Token as follows",
  so its own text needs no keyword to be a rule. Units normative in neither
  version are prose and are not reported.

Sentences would be finer units, but splitting sentences in spec prose breaks
on reference strings ("C. Mortimore") and leaves a lone "REQUIRED." from a
definition; a paragraph is what `read` shows anyway, and the change says
what moved.

## Pairing units

Steps pair with steps and other units with other units, within a pair of
sections, in two passes:

1. **Same content.** Units pair, in order, when their content keys are
   equal. A content key is the unit's words, lower-cased, without
   punctuation or a month followed by a year (other numbers are content),
   and without the reference tooltips an OpenID page renders inline, a
   parenthesis with a quoted title that ends in a year (`[RFC6749] (Hardt,
   D., Ed., “…,” October 2012.)`, the label kept) or a cross-reference's number when its
   title follows (`Section 7.1 (TLS Requirements)`), so a renumbering is no
   change. Reference strings carry a
   publication date ("OpenID Connect Core 1.0, November 2014" became
   "December 2023" in every claim of §2.4), so without this every claim
   would look changed, and in pass 2 the new `exp` would pair with the old
   `iat` it resembles, reporting `iat` as added.
2. **Similar.** The remaining units pair greedily, most similar first, when
   the Dice coefficient of their content-word multisets is at least 0.5,
   and never when both start with a definition term and the terms differ.

Unpaired units are added (in `to` only) or removed (in `from` only).

## Classifying a pair

- Keyword sets differ: `strength_changed`, with both keyword lists
  (OPTIONAL → REQUIRED, SHOULD → MUST, none → MUST).
- Same content key, or the differing words pair off one to one as spelling
  fixes, at least five letters long, one edit apart and neither the other
  with a letter added at an end ("occured" → "occurred"): `reworded`. Two
  edits reach opposites (`signed`, `unsigned`), and the same words in
  another order are a change.
  Listed apart, in `reworded`, not in `changes`.
- Otherwise: `changed`. "Validate the iss, aud, and iat Claims" becoming
  "… iss, aud, iat, and exp Claims" adds a word, so it is a change, though
  no keyword moved.
- Identical text: not reported.

The rule errs toward reporting: a difference it can't show is cosmetic is a
change, since a missed rule costs an agent more than an extra one to read.
Numbers other than years count as content, since "HTTP 400" against "HTTP
401" is normative.

## Result

Each change has `change` (`added`, `removed`, `changed`, `strength_changed`
or `reworded`), the `section` number, `from` and `to` citations, and
`from_text`/`to_text`, `from_strength`/`to_strength` for the sides that
have the statement. `from` and `to` cite where the statement is, or for an
added or removed one, the section in the other version it is missing from,
when that version has the section: so `exp` added in §2.4 cites both
versions' §2.4. Texts are held to the snippet budget. The result names the
document, both pinned versions, the section asked for, the license class
and `indexed_at`; at most 50 changes are listed, with `total`.

`search` and `read` results from the current version of a document with
older pinned versions carry `older_versions`, their pinned citations; the
server instructions point to `diff`.

## Unifying later

#157 adds its own normative-statement extraction. This one lives in one
function, `version_units` in `codecite-core/src/store/diff.rs`, so the two
can be merged into one extractor later without changing what `diff`
reports.

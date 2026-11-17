# Derive chunk text and terms instead of storing them (#249)

**Goal:** stop storing what a corpus can derive. `chunk.terms` goes, and a
chunk of a document that keeps `document.text` stores no text of its own.
Search returns the same ordered citations and snippets as before.

**Architecture:** a chunk's text is a line range of its document's text
whenever the document stores one, so `chunk.text` is NULL there and readers
cut it from `document.text`. Spec and guidance chunks, whose documents keep
no text, keep theirs. Terms are split from the text at insert, as now, and
from the text again when a migration refills the full-text table.

## Storage choice

Two shapes were on the table: NULL `chunk.text` with the span cut from
`document.text` on read, or the same text moved to a narrow `chunk_text`
side table.

NULL plus line ranges, because:

- The side table keeps every byte of chunk text, so it saves only `terms`
  (11 of the ~20 MiB the issue counts on `identity.db`). NULL text saves the
  text as well, and nests no copies: a class chunk no longer repeats its
  methods.
- No new columns are needed. The chunker's text is exactly lines
  `start_line..=end_line` of the file with the last line's `\n` and then
  `\r` dropped (`Lines::span` in `codecite-index`), and `start_line` and
  `end_line` are already stored. Byte offsets or a stored `line_starts`
  would add bytes to save a scan that only the top k results pay.
- It is safe by construction: `insert_chunks` stores NULL only when cutting
  the lines from the document's text gives back the chunk's text byte for
  byte, so any chunk a future chunker cuts differently keeps its text.
- The ranking query gets narrower rows either way. With NULL text, the
  ranked rows carry no code text through the sort; the text is read for the
  `limit` best rows only, after the sort.

## Line cutting

`store/lines.rs`:

- `line_span(text, start, end)` scans to line `end` and stops. The SQL
  function `codecite_lines(document.text, start, end)` wraps it.
- `LineStarts` precomputes every line's offset, for cutting many chunks of
  one document (insert, migration). A test checks both cut the same spans.

Readers use `coalesce(c.text, codecite_lines(d.text, c.start_line,
c.end_line))`. SQLite's `coalesce` stops at the first non-NULL argument, so
a spec chunk never loads its document's text.

## Steps

1. **Test first:** `lines.rs` tests (spans with CRLF, a final newline, past
   the end, backwards; `LineStarts` against `line_span` on edge texts).
2. **Write:** `insert_chunks` drops `terms`, and writes NULL text when the
   document's lines give the chunk's text. Test: a code chunk stores NULL, a
   chunk whose text isn't its lines and a spec chunk keep theirs, every chunk
   reads back as written, and `chunk` has no `terms` column.
3. **Migration 0010:** rebuild `chunk` without `terms` and with nullable
   `text` (SQLite can't drop NOT NULL), keeping IDs, indexes and the
   `chunk_fts_delete` trigger. In Rust, in the same transaction, clear the
   text of every chunk its document's text holds. `fill_fts`, the refill
   versions 5 and 8 run, splits terms from the (coalesced) text. After the
   transaction commits, `VACUUM` a file that came up from an older version,
   so the freed pages leave the file. Test: a version 9 corpus with a
   nested class, a method, a window that no longer matches its lines and a
   spec section comes up with the right NULLs, reads every chunk back,
   keeps its full-text rows, indexes and trigger, and has no free pages.
4. **Readers:** search (inner query ranks without text; the outer reads
   text for the `limit` rows and sorts them again by the same key),
   definitions, context rules, requirements, `read` sections and steps,
   `grep` over a spec and `diff`.
5. **Equivalence:** index the retrieval benchmark corpus
   (`evals/retrieval/identity-bench.toml`) with `main` and with this branch,
   run every query in `evals/retrieval/queries.toml` through
   `codecite search --json --limit 10`, and diff the whole output
   (citations, order, scores and snippets; `indexed_at` aside). It must be
   identical.
6. **Size:** report `identity.db` before and after on fresh builds, and
   `fhir.db` by migrating a copy of the existing file.
7. **Docs:** `docs/design.md` Data model (the `chunk` table), Indexing step
   5 (terms), and the "Identifier splitting" decision.

## Concurrency

Other open work adds migrations (#242, #248). This takes 0010, the next
free number on `main`; whoever lands later renumbers.

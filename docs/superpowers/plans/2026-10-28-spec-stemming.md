# Spec stemming and query stopwords

Issue #156, follow-up 1 of `docs/superpowers/specs/2026-10-23-hybrid-retrieval-design.md`.

## Goal

Spec search gets Porter stemming and drops common English words from the
query. Code search stays as it is.

## Steps

1. **Migration 0005.** Create `spec_fts`, the same columns as `chunk_fts`
   with `tokenize='porter unicode61'`, and a delete trigger for it. Delete
   the spec chunks' rows from `chunk_fts`, and empty the spec documents'
   content hashes so the next index run rewrites them into `spec_fts` (a
   contentless table can't give their terms back). Test: a version 4 corpus
   with a code and a spec chunk keeps only the code row in `chunk_fts`, and
   its spec document's hash is empty.
2. **Write path.** `write_document` reads the source's kind and puts a
   spec's full-text rows in `spec_fts`. Test: a spec document matches in
   `spec_fts`, stemmed ("checked" finds "Checking"), and not in `chunk_fts`;
   deleting the source empties both.
3. **Search.** Rank each table with its own query: code from `chunk_fts`
   with every term, specs from `spec_fts` with `spec_fts_query`. Keep
   `blend` as it is.
4. **Stopwords.** `STOPWORDS`, the spike's list, sorted; `spec_fts_query`
   drops them, and a query of only stopwords keeps them joined with `AND`.
   Tests: the query strings, the list's order, a stemmed spec search that
   ignores "is", an all-stopword query that finds one section, and code
   search that is neither stemmed nor stopworded.
5. **Benchmark.** Run `codecite-bench` on `main` and on the branch over
   `identity-bench`; put both tables in the PR and the spec-requirement
   number in `docs/design.md`.
6. **Docs.** `docs/design.md`: the schema, the diagram and a Ranking bullet.
7. **PR** with the `eval` label.

# Spec stemming and query stopwords

Issue #156, follow-up 1 of `docs/superpowers/specs/2026-10-23-hybrid-retrieval-design.md`.

## Goal

Spec search gets Porter stemming and drops common English words from the
query. Code search stays as it is.

## Steps

1. **Migration 0005.** Create `spec_fts`, the same columns as `chunk_fts`
   with `tokenize='porter unicode61'`, and a delete trigger for it. Spec
   chunks keep their `chunk_fts` rows, so code's BM25 statistics don't move.
   `Store::open_read_write` fills `spec_fts` from the stored spec chunks in
   the same transaction (their path terms are split in Rust), so an upgraded
   corpus needs no new index run. Test: a version 4 corpus's spec chunk is
   in `spec_fts`, stemmed and with its path terms, and nothing is marked for
   rewriting.
2. **Write path.** `write_document` reads the source's kind and puts a
   spec's full-text rows in `spec_fts`. Test: a spec document matches in
   `spec_fts`, stemmed ("checked" finds "Checking"), and in `chunk_fts`;
   deleting the source empties both.
3. **Search.** Rank each table with its own query: code from `chunk_fts`
   with every term, specs from `spec_fts` with `spec_fts_query`. Keep
   `blend` as it is. Test: code results and scores equal the query before
   this change, and spec rows are still in `chunk_fts`.
4. **Stopwords.** `STOPWORDS`, the spike's list without RFC 2119's
   keywords and the negations `not` and `no`, sorted; `spec_fts_query`
   drops them, and a query of only stopwords keeps them joined with `AND`.
   Tests: the query strings, the list's order, a stemmed spec search that
   ignores "is", an all-stopword query that finds one section, and code
   search that is neither stemmed nor stopworded.
5. **Benchmark.** Run `codecite-bench` on `main` and on the branch over
   `identity-bench`; put both tables in the PR and the spec-requirement
   number in `docs/design.md`.
6. **Docs.** `docs/design.md`: the schema, the diagram and a Ranking bullet.
7. **PR** with the `eval` label.

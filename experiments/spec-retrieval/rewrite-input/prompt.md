# The rewrite prompt

`rewrites.json` was written by one model (Claude Sonnet, through a Claude Code
subagent with no repository or web access) given exactly the prompt below and
`rewrite-input.json`: the 55 spec query texts under anonymous ids (`q1`…`q55`),
so the ids gave away neither the query class nor the answer. `anon-map.json`
maps the anonymous ids back to the ids in `evals/retrieval/queries.toml`. One sample, no
retries, no editing.

---

You are simulating a coding agent that is about to call a full-text (BM25 keyword) search tool over a corpus of OAuth 2.0 / OpenID Connect / JOSE specifications (IETF RFCs, OpenID Foundation specs, FAPI 2.0). Keyword search only matches the specs' own words, so before searching, an agent rewrites a user's question into the terms the specs themselves use.

Read rewrite-input.json — a JSON object of id -> question. For each question write ONE search query of 3–10 words: the spec terminology an expert would type (protocol names, parameter names, endpoint names, claim names, error codes, spec titles). Keep any spec terms the question already uses. Don't explain. Work from your own knowledge only: do not search the web, do not open any other file, do not look at any repository.

Write the result to rewrites.json as a JSON object of the same ids -> rewritten query.

# Missed definitions (#312)

What the tree-sitter query changes in #312 do to the indexes they build.

- `counts.py <workdir> <base-binary> <branch-binary> <generic-src>` indexes
  the history benchmark's repositories (at their pinned commits) and the
  generic-retrieval repositories ripgrep, excalidraw, django and keycloak
  with both binaries, and writes `results/counts.md`: symbols by language
  and kind, the rows that changed.
- `parity.py <retrieval-generalization dir> <workdir> <generic-src>
  <base-binary> <branch-binary>` runs every generic-retrieval case on those
  four repositories with both binaries and diffs each `def` and `search`
  call (`results/parity.log`). It is not a gate.

Base: `refactor/generic-ranking-rules` at bca6204. Branch:
`fix/missed-definitions`.

## Results

- **Counts** (`results/counts.md`). The history repositories lose 1–7
  methods each: methods of unnamed object literals (callbacks, options
  objects) that are no longer definitions; terraform-provider-descope and
  ripgrep don't change. excalidraw gains 195 TypeScript/TSX methods (class
  properties holding an arrow function) and 6 TSX functions, the local
  functions inside those methods (`App#…().preventNextPaste()`). django
  gains 189 JavaScript methods, 176 of them `X.prototype.m = function` in
  the admin's vendored select2 and the rest in `core.js` and
  `OLMapWidget.js`. keycloak gains 11 JavaScript methods and the 2
  file-scope C tables (`crypto_sysctl_table`, `crypto_dir_table`) of its
  one C file, a CI helper.
- **Parity** (`results/parity.log`). 114 of 123 calls give identical
  output; the 9 that differ are excalidraw `search` calls, where the new
  method chunks change the rows, and the first right answer keeps its rank
  in every one (8 at rank 1, one unscored case). No call lost a
  hit.
- **History benchmark** (`evals/history/results/`). Recall@1, Recall@10
  and MRR match the base on every repository at the scorecard's precision;
  unrounded, one blind-peer-review query moved from rank 6 to 7 (MRR
  0.5017 → 0.5010).

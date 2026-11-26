# CodeGraph extraction kernel spike

A throwaway spike. Can CodeGraph's MIT-licensed Rust extraction kernel (`codegraph-kernel/`) replace or supplement codecite's own tree-sitter extraction (`crates/codecite-index/src/chunk/code.rs`)? And can it supply the signature and doc-comment "header" text that ranking (#299) lacks? It runs over the five pinned repositories of #287 (`../retrieval-generalization/corpora.toml`). No model, eval or network is in the loop beyond cloning CodeGraph and fetching crates. Production code is unchanged.

## Findings

1. **Don't replace codecite's extraction with the kernel.** The kernel matches codecite's definitions on its own ground:
   - Python 100% (Django: 41,901 of 41,901 functions and types, 2,750 of 2,750 module variables).
   - Java 99.99% (Keycloak: 92,262 matched; 7 codecite-only).
   - TypeScript and TSX 96.9 to 99.8%, JavaScript 89.5 to 97.5%.
   - Rust functions and types 99.97 to 100%.

   It does not match on C, run bare as a Rust library. It **defers** any file whose parse tree contains an error, and so does no extraction there. In CodeGraph those files go to its TypeScript/wasm fallback; a Rust caller gets nothing. On Linux that is 30,963 of 61,991 C files, which hold 698,476 of codecite's 884,227 C definitions. Only **20.8%** of codecite's C definitions come back, though 99.2% do inside the files the kernel extracts. codecite's queries extract from trees with errors, and the kernel's walkers can't. Elsewhere at most 3 files per repository defer.
2. **The disagreements are mostly scope choices, not one side being wrong.** Judged on 10 sampled examples per language and direction (`results/judgements.json`):
   - **The kernel loses:**
     - C `typedef struct tag {..} alias;` keeps only the alias, dropping the tag (`struct xfs_trans`). 10 of 10 C codecite-only examples, codecite right.
     - Java methods of anonymous classes passed as arguments (7 of 7).
     - Rust trait associated consts.
     - TSX props-interface members emitted as methods (8 of 10 TSX kernel-only examples, codecite right).
     - Python temporaries inside module-level `for`/`if`/`__main__` blocks (8 of 10).
     - A Rust `const` named after its initialiser (`Register`, `PhantomData`, `None`) instead of itself (7 of 10 Rust kernel-only examples). That one is a kernel bug.
   - **The kernel wins:**
     - TypeScript class arrow-function properties (`static updateScene = () => ..`). 7 of 10 TypeScript kernel-only examples, kernel right; codecite misses them.
     - JavaScript `X.prototype.m = function`.
     - C function-like `#define` macros and file-scope tables (8 of 10 C kernel-only examples).
     - codecite's own noise: nested object-literal callbacks, generated emscripten getters and minified one-letter functions. 7 of 10 TSX and 7 of 10 TypeScript codecite-only examples, kernel right.
   - **Design choices:** Java `static final` fields (10,388 kernel-only rows on Keycloak) and Rust function-local consts are policy, not error.

   None of these needs the kernel. Each is a small query change on codecite's side if wanted (#269-style generic fixes).
3. **The kernel's header fields are good everywhere but C.** For the kernel's functions, methods and types, a signature is present for:
   - Java 88%, Rust 74 to 87%, Python 74 to 85%, TypeScript 75 to 76%, TSX 70 to 72%.
   - C **0%**: the C walker emits no parameter list at all.

   A doc comment or docstring is present for Rust 27 to 38%, Python 22 to 27%, C 24%, Java 17%, TypeScript 8 to 19% and TSX 1 to 5%: that is how often the source has one. Judged for ranking, 10 per language: Java, Python and TSX 10 of 10 usable; Rust, TypeScript and JavaScript 9 of 10; C 4 of 10. The rest are partial: C missing its parameters, TypeScript generics dropped, a Rust `///` block above `#[repr]` missed, a JavaScript line comment missed. Java's signature field starts with the return type (`void (String x)`).
4. **The kernel adds no language coverage on these corpora.** Its kernel-only languages account for 10 files across all five repositories: Linux 7 C++ and 1 Ruby, ripgrep 1 Lua and 1 Ruby. CodeGraph would also route some Linux `.h` files to C++ by content sniffing, and that is TypeScript-side too.
5. **Cost is a wash at runtime but heavy in size.**
   - **Time:** single-threaded, the kernel takes 0.29 / 3.09 / 0.78 / 8.84 / 97.5 s on ripgrep / Django / Excalidraw / Keycloak / Linux. codecite's chunker takes 0.28 / 2.57 / 0.60 / 6.35 / 113.3 s, and a whole `codecite index` 0.61 / 7.38 / 3.48 / 19.4 / 225 s. Linux's kernel time skips the walk for the deferred half.
   - **Memory:** peak RSS is 16 to 51 MiB against codecite-extract's 23 to 61 MiB.
   - **Binary size:** the kernel alone is a **37.6 MB** stripped binary (`kernel-extract`); its 20 grammars include Scala's and Swift's 20 to 27 MB generated parsers. codecite today is 27.0 MB stripped.
6. **Wiring it in is possible, but there is a silent trap.**
   - **Two tree-sitter versions:** one binary can't link both: tree-sitter 0.25 and 0.27 both declare `links = "tree-sitter"`, so Cargo refuses to resolve (`two-versions-probe/`).
   - **Porting the kernel to 0.27 is mechanical.** `bump-ts027.sh` changes 21 files, mostly `named_child(i)` taking `u32` and the progress callback returning `ControlFlow`. The ported kernel's output is **byte-identical** on all five repositories (sorted node lines, 960,405 nodes, same deferred files). One upstream test fails: 0.25's endless TSX error recovery no longer reproduces, so its timeout never fires.
   - **The trap:** codecite-index and the ported kernel in one binary build and run, but each grammar exports one C symbol per language (`tree_sitter_python`). With codecite on tree-sitter-python 0.25 and the kernel on 0.23 (Go likewise), the final binary held **one** `tree_sitter_python`. The linker silently picked one, so one side parses with a grammar its queries or walkers weren't written for. Any in-process use must pin one version of every shared grammar crate and retest both sides on it.

## Recommendation

Neither replace nor supplement now. For #299, compute the header in codecite from the nodes it already has, with no new dependency:

- the definition's text up to its body (name, parameters, return type, generics); and
- the doc-comment lines `documented_start` already folds into the chunk (Rust `///`, JSDoc, Javadoc, C comments; Python docstrings from the body's first statement).

The kernel shows the payoff is real where the source has it, and its C gap shows the signature has to come from the grammar node, not a per-language walker.

If codecite later needs languages it has no grammar for (C++, Ruby, PHP, Kotlin, Swift, Scala, Dart, Lua, R), take shape (b) below, side by side, over shape (a), for the reasons there.

## Integration shapes (sketches, no production change)

**(a) In-process.** codecite-index depends on a kernel crate ported to 0.27. In `index_tree`, for a file codecite has no grammar for:

```rust
match codegraph_kernel::api::extract(&path, &text, cg_language, Some(10_000)) {
    Ok(x) => for n in x.nodes.iter().filter(|n| DEFINITION_KINDS.contains(&n.kind)) {
        chunks.push(Chunk { kind: chunk_kind(n.kind), start_line: n.start_line, end_line: n.end_line,
                            text: lines.span(n.start_line, n.end_line),
                            symbol: Some(Symbol { name: n.name.clone(), kind: symbol_kind(n.kind),
                                                  descriptor: descriptor_from(&n.qualified_name) }), .. });
        headers.push(header(n.signature.as_deref(), n.return_type.as_deref(), n.docstring.as_deref()));
    },
    Err(ExtractError::Defer(_) | ExtractError::Timeout(_)) => { /* windows only, as for an unknown language */ }
    Err(ExtractError::Other(e)) => return Err(..),
}
```

The header text then goes into a new `chunk_fts` column (`header`, weighted in `bm25()` between `symbol` and `terms`), or into a re-rank input. Costs:

- every kernel grammar and walker in codecite's binary (about +37 MB);
- one version of every shared grammar crate (above);
- a fork to track, since the kernel is `publish = false` and napi-bound;
- CodeGraph's 20-language walker churn landing in codecite's release cycle;
- `CHUNKER_VERSION` bumped whenever the kernel changes.

**(b) Side by side.** A separate `codegraph-extract` binary (this spike's `kernel-extract`, grown up) writes a versioned interchange file per source:

```json
{"producer": "codegraph-kernel", "version": "...", "source_sha": "<HEAD>", "files": [{"path": "...", "sha256": "...", "language": "kotlin",
  "definitions": [{"kind": "method", "name": "...", "qualified_name": "...", "start_line": 1, "end_line": 9, "signature": "...", "doc": "..."}]}]}
```

The manifest names it per source, beside `scip`. Like the SCIP import, codecite verifies each file's hash against the walked checkout, then turns rows into chunks and symbols with provider `codegraph` for files that have no codecite grammar.

- It keeps tree-sitter versions, grammar symbols and binary size out of codecite.
- It reuses the hash-verified, provider-tagged import path #295 already argued for (`../graph-reuse/README.md`).
- It lets the producer run per language only where needed.

Recommended over (a) for both reasons: no grammar-symbol coupling, and no 37 MB in every install for languages most corpora don't have.

## Method

- `codecite-extract/` (path-depends on `crates/codecite-index`, tree-sitter 0.27) walks each checkout with codecite's own `walk_local` and chunks every kept file with `Chunker`. It writes the file list and every chunk that has a symbol. That gives codecite's definitions exactly as `codecite index` makes them, without the store.
- `kernel-extract/` (path-depends on the fork through the `fork` symlink, tree-sitter 0.25) runs `codegraph_kernel::api::extract` on the same file list. It routes by codecite's extension table first, then CodeGraph's for kernel-only languages. It runs bare: no TS-side `preParse` (C/C++ macro blanking), no `.h` sniffing, no wasm fallback. Files with no extension that codecite detects by shebang are not routed; they are counted apart.
- `compare.py` matches within a (path, name): a kernel node pairs with a codecite definition when its start line falls inside codecite's chunk, which starts at the doc comment. The cheapest start-plus-end distance wins, one to one.
  - Matched pairs agree on the end line within 1 in every case except Rust variables.
  - The kernel's kinds are compared in two buckets: functions, methods and types; and variables and constants. Imports, fields, properties, enum members and namespaces are left out.
  - Codecite-only definitions in deferred and in shebang files are counted separately.
  - Samples are drawn with seed 20261125 over sorted keys.
  - Judgements were made against the pinned source by reading each sampled item in context. They are in `results/judgements.json`, per sample, with the pattern per section.
- `run.sh` runs both extractors, the kernel again on all 22 threads, and a full `codecite index`, serially, under `unshare -rn`, with `/usr/bin/time` for wall time and peak RSS. The codecite build is this branch's base (`62d9f57`), release profile.

## Results

- `results/summary.md` holds the parity, kernel-file, header and cost tables per repository and language.
- `results/parity.json`, `headers.json`, `cost.json` and `coverage.json` hold the same tables as JSON.
- `results/samples.md` holds the sampled disagreements and headers with source lines, and `results/judgements.json` the verdicts.

Raw JSONL (about 260 MB for Linux) stays in the run directory.

## Fork changes

Upstream: `https://github.com/colbymchenry/codegraph` at `73c6cff66e525a92bff65675b6b0c46687e91ef1` (2026-11-23). The local fork is `~/repos/codegraph-fork`, branch `local/rlib-extraction`, and it has never been pushed. `fork-rlib.patch` is that branch's one commit:

1. `Cargo.toml`: `crate-type = ["cdylib", "rlib"]`, plus a `napi` feature (default on). `napi`, `napi-derive` and `napi-build` become optional dependencies behind it.
2. `build.rs`: `napi_build::setup()` only with the `napi` feature.
3. `src/lib.rs`: the napi items move verbatim to `src/node.rs` (`#[cfg(feature = "napi")] mod node;`), and lib.rs declares `pub mod api;`.
4. `src/api.rs` (new): `extract(path, content, language, budget) -> Result<Extraction, ExtractError>`. It runs the same walker dispatch as `extract_file`, then decodes the flat wire buffers into owned `Node`, `Edge` and `Ref` structs; `ExtractError` is `Defer`, `Timeout` or `Other`. It has one test.

With defaults, `cargo build --release` still builds the napi addon. `cargo test --no-default-features` passes all 31 kernel tests. CodeGraph's MIT `LICENSE` stays in the fork; nothing from it is copied into this repository except the patch text.

`bump-ts027.sh` is the second local branch, `local/ts-0.27`: the 0.27 port.

## What an upstream request would ask for

A napi-free extraction crate published alongside the addon:

- a `codegraph-extract` (or `codegraph-kernel` with `default-features = false`) library crate on crates.io;
- a typed per-file API like `api::extract` above, instead of only the wire buffers;
- the napi bindings as a thin crate on top of it.

Nice to have:

- a tree-sitter version that tracks upstream (0.26/0.27), or a semver range;
- grammar crates selectable by feature, so a consumer can drop the 20 to 27 MB grammars it doesn't need and align shared ones;
- a documented contract for deferred files (a Rust consumer has no wasm fallback);
- the three walker bugs above:
  - the C typedef tag;
  - the Rust `const` name taken from its initialiser;
  - the C signature always empty.

## Reproduce

```sh
git clone https://github.com/colbymchenry/codegraph ~/repos/codegraph-fork
git -C ~/repos/codegraph-fork checkout -b local/rlib-extraction 73c6cff66e525a92bff65675b6b0c46687e91ef1
git -C ~/repos/codegraph-fork am "$PWD/experiments/codegraph-kernel/fork-rlib.patch"
cd experiments/codegraph-kernel
ln -sfn ~/repos/codegraph-fork fork
(cd kernel-extract && cargo build --release --locked)
(cd codecite-extract && cargo build --release --locked)
(cd ../.. && cargo build --release --locked -p codecite)
# $src/<repo>: clean checkouts at the pins in ../retrieval-generalization/corpora.toml
./run.sh "$run" "$src" ../../target/release/codecite
python3 -I compare.py "$run" "$src" results
```

For the 0.27 check, run `bump-ts027.sh <fork checkout>` on a second checkout of `local/rlib-extraction`. Then point a copy of `kernel-extract` at it and compare sorted `kernel.jsonl`. On this host (22 threads) `run.sh` took about 9 minutes, Linux nearly all of it.

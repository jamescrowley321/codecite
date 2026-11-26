# Definitions codecite misses in TypeScript, JavaScript and C (#312)

**Goal:** `codecite def` finds three kinds of definition the CodeGraph
kernel spike (#311) showed it missing, and stops indexing the object-literal
callbacks the spike judged noise. Every change is a tree-sitter query
pattern written as node structure; no rule names a project, a path or a
file.

**Evidence:** `experiments/codegraph-kernel/results/samples.md` and
`judgements.json` (seed 20261125). The verdicts this plan acts on:

| Sample set | Verdict pattern | Change |
| --- | --- | --- |
| typescript/kernel_only 2, 3, 5, 6, 10; tsx/kernel_only 3, 9 | class property initialised with an arrow function | Task 2 |
| javascript/kernel_only 4, 10 | `X.prototype.m = function` | Task 3 |
| c/kernel_only 1, 4, 10 | function-like `#define` | Task 4 |
| c/kernel_only 2, 3 | file-scope table (`static … x[] = {…}`, `static struct s x = {…}`) | Task 4 |
| tsx/codecite_only 2, 3, 9; typescript/codecite_only 2, 5–9 | method in an object literal passed as an argument, held in an array or nested in another object (JSX `onClick() {}`, `Object.defineProperty(…, { get() {} })`, `defineConfig({ … resolveId() {} })`) | Task 5 |

**Kept, with the reason:**

- **Minified one-letter functions** (javascript/codecite_only 2, 3, 9). No
  node structure tells `function b(u){…}` in a minified bundle from a real
  one-letter definition, such as a translation helper `t`; a name-length
  rule would drop those too. The fix belongs to file detection (a minified
  file windowed instead of chunked), not to the queries, so it stays out of
  this issue.
- **Method signatures in a props type** (tsx/codecite_only 4, 6, 10:
  `onClick?(): void;`). A `method_signature` in a React props type has the
  same node structure as one in a service interface, which is the declared
  API and which codecite keeps for Go, Java and C# interfaces too
  (`interface_methods_without_bodies`). They stay.
- **Function-local type aliases** (tsx/codecite_only 7,
  typescript/codecite_only 4) and **arrow members of a module-level object**
  (typescript/kernel_only 7, 9) are outside the issue's list; left as they
  are.
- **Object-like `#define`** (`#define REG 0x1c8`): the issue names
  function-like macros. A kernel tree holds hundreds of thousands of
  object-like ones, and they would swamp `def` and the index. Left out.

**Design of each change:**

- **Class property with an arrow function.** TypeScript's grammar has
  `(public_field_definition name: … value: (arrow_function))`; JavaScript's
  has `(field_definition property: … value: (arrow_function))`. Capturing it
  as `@function` inside a class makes it a method (`API#updateScene().`),
  the same as a method definition, through the existing `in_type` rule in
  `code.rs`. A `function_expression` value counts too.
- **Prototype method.** `(expression_statement (assignment_expression
  left: (member_expression object: (member_expression object: (identifier)
  @receiver property: (property_identifier) @_proto) property:
  (property_identifier) @name) right: [(function_expression)
  (arrow_function)]) (#eq? @_proto "prototype")) @function`. The
  `@receiver` capture already exists for Go methods: it makes the
  descriptor `Defaults#set().` and the kind a method. `prototype` is a
  language keyword, not a project's name. Both the JavaScript and the
  TypeScript query get it.
- **Function-like macro.** `(preproc_function_def name: (identifier)
  @name) @function`: a `function` chunk and symbol `PLANE_AUX_OFFSET().`.
- **File-scope table.** A `declaration` whose `init_declarator` value is an
  `initializer_list`, at file scope: directly under `translation_unit` or
  under a `preproc_if`/`preproc_ifdef`/`preproc_else`/`preproc_elif` there
  (a table inside `#ifdef CONFIG_X`). Its name sits under array and
  pointer declarators (`*const names[]`), so a new `@c_variable` capture
  on the `declaration` with `@c_declarator` on the `init_declarator` reuses
  the declarator walk `c_function` already does, moved into a helper. Kind
  `Variable`, chunk kind `variable`, symbol `id_table.`. A scalar
  (`static int n = 0;`) is not a table and stays out; so does a
  declaration without an initialiser and an `extern`.
- **Object-literal methods.** `(method_definition name: (_) @name)
  @function` matches anywhere today. It becomes:
  - in a class: `(class_body (method_definition name: (_) @name) @function)`;
  - in an object literal that has a name: the object is the `value` of a
    `variable_declarator`, the `right` of an `assignment_expression`
    (`module.exports = {…}`, `X.prototype = {…}`), or the `value` of an
    `export_statement` (`export default {…}`); TypeScript adds the same
    object under `as_expression` or `satisfies_expression` (`{…} as const`).

  A method in any other object literal (an argument, an array element, a
  JSX attribute, a nested object) is no longer a definition. Its lines stay
  in the chunk of the definition around it, so search still finds the text.

**Learn notes** (the owner is learning Rust through this project):

- Tree-sitter predicates such as `#eq?` filter a match by its captured
  text; the Rust binding applies them inside `QueryCursor::matches`, given
  the source bytes. A capture whose name starts with `_` is a convention for
  "only used by a predicate"; `definitions()` ignores names it doesn't know.
- An alternation `[ … ]` at the top of a pattern lists whole patterns; a
  parent with a choice of kinds is written as one pattern per parent.

**Chunker version:** the same file splits differently, so
`CHUNKER_VERSION` goes from 7 to 8 (Task 6), and every corpus re-chunks on
its next index.

## Task 1: fixtures and failing tests

Files: `crates/codecite-index/src/chunk.rs` (tests module).

Write the tests first; each fails on the current queries.

- `typescript_class_arrow_properties_are_methods`: a `.ts` file with
  `class API { static updateScene = (a: number) => {\n};\n private
  helper = function () {};\n count = 0;\n }`. Expected: `API#`,
  `API#updateScene().`, `API#helper().` as `function` chunks; `count` has
  none. The same text as `.tsx` gives the same outline. A `.js` file with
  `class C { handle = () => {}; }` gives `C#handle().`.
- `javascript_prototype_methods_are_methods`: `function Defaults() {}\n
  Defaults.prototype.set = function (k, v) {\n};\nDefaults.prototype.apply
  = (o) => o;\nDefaults.other = function () {};\n`. Expected:
  `Defaults().`, `Defaults#set().`, `Defaults#apply().`; `Defaults.other`
  gives nothing (not a prototype). Run on `.js` and `.ts`.
- `c_function_like_macros_and_file_scope_tables`: `#define REG 0x1c8\n
  #define OFFSET(p, q) ((p) + (q))\nstatic const struct id ids[] = {\n
  { 1 },\n};\nstatic struct clk src = {\n .rate = 1,\n};\nstatic const
  char *const names[] = { "a" };\nstatic int count = 0;\nextern int
  table[];\n#ifdef CONFIG_X\nstatic int guarded[] = { 1 };\n#endif\nvoid
  f(void) {\n static int local[] = { 1 };\n}\n`. Expected: `OFFSET().`
  (function), `ids.`, `src.`, `names.`, `guarded.` (variable), `f().`;
  no chunk for `REG`, `count`, `table` or `local`.
- `object_literal_methods_need_a_named_object`: a `.ts` file with
  `export const api = {\n  get(id) {},\n};\nmodule.exports = { run() {} };\n
  export default { mount() {} };\nconst c = { m() {} } as const;\n
  register({\n  perform() {},\n});\nObject.defineProperty(M, "x", { get()
  {} });\nconst list = [{ onClick() {} }];\nconst nested = { inner: {
  deep() {} } };\n`. Expected methods: `api.get().`, `run().`, `mount().`,
  `c.m().` (descriptor as the chunker builds it today; assert what the
  existing nesting gives), and none for `perform`, `get` under
  `defineProperty`, `onClick` or `deep`. The top-level variables keep
  their own chunks. The JavaScript form (without `as const`) gives the
  same.
- The existing fixtures (`typescript_fixture`, `javascript_fixture`,
  `named_class_expressions`, `c_fixture`) must keep passing unchanged.

Run `cargo test -p codecite-index chunk::` and see the new tests fail.

## Task 2: class properties with an arrow function

Files: `queries/typescript.scm`, `queries/javascript.scm`.

Add `(public_field_definition name: (_) @name value: [(arrow_function)
(function_expression)]) @function` to TypeScript and the
`field_definition property: …` form to JavaScript. Confirm the field names
against the grammar by running `every_query_compiles` (a wrong field name
is a query error). The TypeScript test passes; commit
`fix(index): index class properties holding a function (#312)`.

## Task 3: prototype methods

Files: `queries/javascript.scm`, `queries/typescript.scm`.

Add the prototype pattern (Design above) to both. Check in
`definitions()` that `@receiver` on a JavaScript match gives the
`Defaults#set().` descriptor; nothing else in Rust should need to change.
Commit `fix(index): index methods assigned to a prototype (#312)`.

## Task 4: C function-like macros and file-scope tables

Files: `queries/c.scm`, `chunk/code.rs`.

1. Add `(preproc_function_def name: (identifier) @name) @function`.
   `symbol_kind` maps `preproc_function_def` to `Function` by default; check.
2. Move the declarator walk out of the `"c_function"` arm into
   `fn c_declarator_name(node: Node<'_>) -> Option<Node<'_>>` (follows the
   `declarator` field, through `parenthesized_declarator`, to the
   `identifier`). Learn: returning `Option<Node<'tree>>` with the node's
   lifetime lets the caller keep the name node without copying text.
3. Add the `@c_variable`/`@c_declarator` arms: `c_variable` sets the node
   and `captured_kind = Some(SymbolKind::Variable)`; `c_declarator` sets the
   name through `c_declarator_name`.
4. Add the table patterns to `c.scm`, one per parent kind
   (`translation_unit`, `preproc_if`, `preproc_ifdef`, `preproc_else`,
   `preproc_elif`), each `(declaration declarator: (init_declarator value:
   (initializer_list)) @c_declarator) @c_variable` under it.
5. The C test passes, the forward-typedef tests too. Commit
   `fix(index): index C function-like macros and file-scope tables (#312)`.

## Task 5: object-literal methods only in a named object

Files: `queries/typescript.scm`, `queries/javascript.scm`.

Replace the bare `method_definition` pattern with the class-body pattern
and the named-object patterns (Design above). Run the whole chunk test
module: the class fixtures and `named_class_expressions` must not move.
Commit `fix(index): drop methods of unnamed object literals (#312)`.

## Task 6: chunker version and docs

Files: `crates/codecite-index/src/chunk.rs`, `docs/design.md`.

- `CHUNKER_VERSION` 7 → 8, with the doc comment noting why.
- `docs/design.md` Write step 3 (Chunk): a class property holding a
  function and a method assigned to a prototype are methods; an
  object-literal method is a definition only in an object a variable,
  assignment or `export default` names; C chunks function-like macros and
  file-scope tables (a declaration with a brace initialiser at file scope,
  inside a preprocessor conditional too). Check `docs/usage.md` for any
  sentence the change makes stale (the `def` section).
- Commit `docs: describe the new TypeScript, JavaScript and C definitions (#312)`.

## Task 7: gates, history benchmark and definition counts

1. Gates: `cargo fmt --check && cargo clippy --workspace --all-targets
   --locked -- -D warnings && cargo test --workspace --locked && cargo deny
   check`.
2. Real CLI: index a scratch `local` source holding the four fixtures with
   the release binary; `codecite def` finds `updateScene`, `set`,
   `OFFSET`, `ids`, and finds nothing for `perform`. Adversarial: a
   malformed C file (unclosed initializer) and a JS file with
   `a.prototype[x] = function(){}` (computed property) still index without
   error.
3. History benchmark: release build, `evals/history/run.sh` in the
   background; commit `evals/history/results/`. Compare each repository's
   pooled `all` rows for `codecite` with `scorecard.md` on
   `origin/refactor/generic-ranking-rules`; a lower Recall@1, Recall@10 or
   MRR on any repository parks the issue.
4. Definition counts: for each history repository, count the tree-sitter
   symbols by kind in the base binary's index and the branch's (one SQL
   `count(*)` over `symbol` joined to the source, by language), and the
   same on the generic repositories the #300 parity script uses (ripgrep,
   excalidraw, django, keycloak) where the TypeScript and JavaScript
   changes show. Write the table to
   `experiments/missed-definitions/results/counts.md` with the script that
   made it, and commit.
5. Generic retrieval parity (non-gating): run the #300 parity script with
   both binaries and record which calls changed and why.

# Docusaurus documentation site Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Docusaurus site in `website/` that renders `README.md`, `docs/usage.md`, `docs/design.md` and `evals/README.md` in place, fails CI on a broken internal link or anchor, and deploys to GitHub Pages from `main` (#291).

**Architecture:** One docs-plugin instance rooted at the repository (`path: '..'`) with an `include` list, CommonMark parsing (`markdown.format: 'detect'`), Mermaid, local search, and a local remark plugin that points links to existing, non-Markdown repository files at GitHub and leaves every other link for Docusaurus to check. One workflow builds on every PR and deploys on `main` through `actions/deploy-pages`. Nothing is copied out of `docs/`.

**Tech Stack:** Docusaurus (the current major, classic preset, TypeScript config), `@docusaurus/theme-mermaid`, `@easyops-cn/docusaurus-search-local`, Node LTS, GitHub Actions, GitHub Pages.

**Spec:** `docs/superpowers/specs/2026-11-23-docusaurus-site-design.md`.

## Global Constraints

- James answers the spec's open questions before Task 1. The tasks below assume its recommendations: no custom domain (`baseUrl: '/codecite/'`), unversioned, classic theme, `design.md` and `evals/README.md` published. A different answer changes only the config values named in Tasks 2 and 3.
- No living doc changes to suit the site. If the build finds a real broken link or anchor in `docs/`, fix the doc in its own commit, since that is drift on GitHub too.
- Specs and plans under `docs/superpowers/` are never published and never edited.
- Every action pinned by full commit SHA with a version comment. `permissions: {}` at workflow level; jobs ask for what they need.
- `npm ci --ignore-scripts` everywhere. No `actions/setup-node` cache in `docs.yml`.
- Job ids are new (`docs-build`, `docs-deploy`); no existing job or required check is renamed.
- No status words; issues by number.

## Review Focus

- A `<corpus>` or `{...}` in prose renders as text, not as a JSX or MDX error.
- A broken internal link (to a missing page, a missing non-Markdown file or unpublished Markdown), or a link to a missing `#anchor` in `usage.md`, fails `npm run build`; the remark plugin rewrites only links to files that exist and are not Markdown.
- A push to `main` that changes no site input leaves `docs-build` green and skips `docs-deploy`.
- `LICENSE` and `NOTICE` links from the README land on GitHub, not on a 404.
- `docs/superpowers/**`, `docs/cloud-sessions.md` and `AGENTS.md` produce no page.
- zizmor passes on `docs.yml` with no new ignore in `.github/zizmor.yml`.
- `docs-deploy` is the only job with `pages: write` or `id-token: write`.

---

### Task 1: Scaffold `website/`

**Files:**
- Create: `website/package.json`, `website/package-lock.json`, `website/.nvmrc`, `website/docusaurus.config.ts`, `website/sidebars.ts`, `website/tsconfig.json`, `website/src/css/custom.css`
- Modify: `.gitignore`

- [ ] Scaffold with `npx create-docusaurus@<current> website classic --typescript` in a scratch directory, then keep only the files above; drop the blog, the example docs, the React landing page and its images.
- [ ] Pin Node: `.nvmrc` holds the current LTS major; `package.json` `engines.node` matches.
- [ ] Add `@docusaurus/theme-mermaid` and `@easyops-cn/docusaurus-search-local` at exact versions; `npm install`, then commit `package-lock.json`.
- [ ] Check that `rm -rf node_modules && npm ci --ignore-scripts && npm run build` succeeds on the empty scaffold. If a package needs its install script, record which and why in the PR body, and only then drop `--ignore-scripts` for it.
- [ ] `.gitignore`: `website/node_modules/`, `website/build/`, `website/.docusaurus/`.
- [ ] Commit: `build(website): scaffold a Docusaurus site`.

### Task 2: Render the living docs in place

**Files:** Modify `website/docusaurus.config.ts`, `website/sidebars.ts`.

- [ ] Configure the classic preset's docs as in the spec: `path: '..'`, `include: ['README.md', 'docs/usage.md', 'docs/design.md', 'evals/README.md']`, `exclude: ['**/node_modules/**', 'website/**', 'docs/superpowers/**']`, `routeBasePath: '/'`, `editUrl: 'https://github.com/jamescrowley321/codecite/edit/main/'`. `blog: false`.
- [ ] `markdown: { format: 'detect', mermaid: true, hooks: { onBrokenMarkdownLinks: 'throw' } }` (on a Docusaurus release before `markdown.hooks`, the top-level `onBrokenMarkdownLinks: 'throw'`); `themes: ['@docusaurus/theme-mermaid', ...]`; `onBrokenLinks: 'throw'`, `onBrokenAnchors: 'throw'`.
- [ ] `url: 'https://jamescrowley321.github.io'`, `baseUrl: '/codecite/'`, `trailingSlash: false`, `organizationName`/`projectName` for Pages.
- [ ] `sidebars.ts`: one sidebar, in order `README` (label Overview), `docs/usage` (Using codecite), `docs/design` (Design), `evals/README` (Evaluation), then a link item "Design specs and plans" to `https://github.com/jamescrowley321/codecite/tree/main/docs/superpowers`.
- [ ] Navbar: title `codecite`, a GitHub link. Footer: the Apache-2.0 notice linking to `LICENSE` on GitHub.
- [ ] `npm run build`. Expect failures only on `LICENSE` and `NOTICE` (Task 3). Check by eye that `/`, `/docs/usage`, `/docs/design` and `/evals` render, the Mermaid diagrams draw, and a table with `<corpus>` in it shows the text.
- [ ] Check that no page exists for `docs/superpowers/**`, `docs/cloud-sessions.md` or `AGENTS.md` (`find build -name '*.html'`).
- [ ] Commit: `feat(website): render README, usage, design and the evals README in place`.

### Task 3: Rewrite links to repository files, add search

**Files:**
- Create: `website/src/remark/repo-links.ts`, `website/src/remark/repo-links.test.ts`
- Modify: `website/docusaurus.config.ts`, `website/package.json`

- [ ] Write failing tests (Node's built-in `node:test`, no new test framework) against a temporary fixture directory standing in for the repo root, holding `LICENSE`, `NOTICE`, `crates/codecite/src/main.rs`, `docs/usage.md` and `AGENTS.md`:
  - a relative link to `LICENSE`, `NOTICE` or `crates/codecite/src/main.rs` becomes `https://github.com/jamescrowley321/codecite/blob/main/<path resolved from the doc's directory>`; a link to the `crates/` directory becomes `.../tree/main/crates`;
  - `LICENSE#L1` becomes `.../blob/main/LICENSE#L1`, fragment kept; `LICENSE?plain=1` resolves `LICENSE` for the lookup;
  - a link to a missing file (`docs/missing.md`, `crates/gone.rs`, `LICENCE`) is left untouched;
  - a link to unpublished Markdown that exists (`AGENTS.md`) is left untouched;
  - a link to a published page (`docs/usage.md`), to `docs/usage.md#search`, an in-page `#anchor` and an absolute or protocol-relative URL are left untouched;
  - a `../` that climbs above the repo root is left untouched;
  - a reference-style link (`[license][l]` with `[l]: LICENSE`) is rewritten through its `definition` node, and a reference-style link to a missing file is not.
- [ ] Implement the plugin as a remark transformer over `link` and `definition` nodes, using the doc's file path from the vfile and a `repoRoot` option (the absolute repository root, so tests can point it at the fixture). For each URL: skip it if it has a scheme, starts with `//` or `#`; split off `#fragment` and `?query`; decode and resolve the path against the doc's directory; skip it if it leaves `repoRoot`, does not exist (`fs.existsSync`), or ends in `.md`/`.mdx`; otherwise rewrite to `blob/main/<path>` (or `tree/main/<path>` for a directory) with the fragment re-attached. Everything skipped is left for Docusaurus to resolve or report, so no list of published pages is needed in the plugin.
- [ ] Register it under the docs preset's `beforeDefaultRemarkPlugins`, so Docusaurus's own link resolution never sees a rewritten link.
- [ ] Add `@easyops-cn/docusaurus-search-local` to `themes` with `indexDocs: true`, `indexBlog: false`, `docsRouteBasePath: '/'`, `hashed: true`.
- [ ] `npm test` and `npm run build` both pass; search finds `pins_stale` and `cite check`.
- [ ] Prove the link check: in an uncommitted scratch edit, add a link to `#no-such-heading` in `docs/usage.md`, see `npm run build` fail naming it, then `git checkout docs/usage.md`. Do the same, one at a time, with a link to `docs/missing.md`, to `crates/gone.rs` (a missing non-Markdown file, which the plugin must leave for Docusaurus to report), to `docs/usage.md#no-such-heading` and to `AGENTS.md` (unpublished Markdown); each must fail the build.
- [ ] Commit: `feat(website): point repository-file links at GitHub and add local search`.

### Task 4: Build and deploy workflow

**Files:** Create `.github/workflows/docs.yml`.

- [ ] Triggers: `pull_request` (no `branches:` filter, so stacked PRs build too), `push` to `main`. `permissions: {}`. `concurrency: { group: pages-${{ github.ref }}, cancel-in-progress: ${{ github.event_name == 'pull_request' }} }` at workflow level; the deploy job gets its own `concurrency: { group: pages, cancel-in-progress: false }`.
- [ ] `docs-build` (`name: docs-build`), `permissions: contents: read`: checkout (`persist-credentials: false`, `fetch-depth: 0` only if the paths step needs history); a step (`id: paths`) that lists the changed files and matches them against `website/**`, `README.md`, `docs/*.md`, `evals/README.md`, `.github/workflows/docs.yml`, writing `relevant=true|false` to `$GITHUB_OUTPUT`. On `pull_request` it diffs `github.event.pull_request.base.sha...github.event.pull_request.head.sha`; on `push` it diffs `github.event.before..github.sha`, and an all-zero or unfetchable `before` counts as relevant. The SHAs reach the script through `env:`, never `${{ }}` inside `run:`. Expose it as the job output `relevant: ${{ steps.paths.outputs.relevant }}`. Every later step carries `if: steps.paths.outputs.relevant == 'true'`, so when nothing relevant changed the job ends green without building and the check can be required; otherwise `actions/setup-node` with `node-version-file: website/.nvmrc` and no cache, `npm ci --ignore-scripts`, `npm test`, `npm run build` (all `working-directory: website`). On `push` to `main` only, `actions/upload-pages-artifact` with `path: website/build`.
- [ ] `docs-deploy` (`name: docs-deploy`), `needs: docs-build`, `if: github.event_name == 'push' && needs.docs-build.outputs.relevant == 'true'`, so a push that changed no site input skips the deploy rather than failing it for want of an artifact, `environment: { name: github-pages, url: ${{ steps.deploy.outputs.page_url }} }`, `permissions: { pages: write, id-token: write }`, one step: `actions/deploy-pages`.
- [ ] Pin every action by SHA with a version comment, using the SHAs the other workflows already use where they share an action (`actions/checkout`).
- [ ] Check both branches of the gate on the PR's own runs or with `act`: a change only under `crates/` gives a green `docs-build` with the build steps skipped and, on `main`, a skipped `docs-deploy`; a change to `docs/usage.md` builds, and on `main` uploads and deploys.
- [ ] Run `zizmor .github/workflows/docs.yml` (the same version `security.yml` runs) with the repo's `.github/zizmor.yml`; fix every finding rather than ignoring it.
- [ ] `actionlint` if available. Commit: `ci(docs): build the site on every PR and deploy it to Pages from main`.

### Task 5: Security gates

**Files:** Modify `.github/dependabot.yml`, `.github/workflows/osv-scanner.yml` (comment only), `.github/workflows/codeql.yml` (if James agreed, open question 6).

- [ ] Dependabot: add the `npm` entry for `/website` with the same schedule, 7-day cooldown, `open-pull-requests-limit: 5`, `commit-message.prefix: "fix(deps)"` and a `website-minor-patch` group as the existing npm entry.
- [ ] OSV-Scanner: update the header comment to name `website/package-lock.json`; confirm in the PR's own OSV run that the lockfile is scanned (it appears in the job log).
- [ ] dependency-review: no change; confirm in the PR's run that `website/package-lock.json` is diffed.
- [ ] CodeQL: add `javascript-typescript` to the matrix if agreed. The new check is `Analyze (javascript-typescript)`; no existing name changes.
- [ ] Commit: `ci(security): cover the website's npm tree in Dependabot, OSV-Scanner and CodeQL`.

### Task 6: Living docs

**Files:** Modify `AGENTS.md`, `README.md`, `docs/design.md`.

- [ ] `AGENTS.md` "Where things live": a row for the documentation site, `website/`, saying it renders `README.md`, `docs/usage.md`, `docs/design.md` and `evals/README.md` in place, so edits go to those files.
- [ ] `README.md` Documentation list: the site URL first.
- [ ] `docs/design.md` Security and privacy, supply chain: the site's npm tree is one lockfile under `website/`, installed without scripts, covered by dependency review, OSV-Scanner and Dependabot; Pages deploys through OIDC with no secret.
- [ ] Build the site once more so the new links are checked. Commit: `docs: point at the documentation site`.

### Task 7: oss-admin follow-up and the PR

- [ ] Open the PR against `main` with `Closes #291`, the acceptance criteria from #291, the AI provenance section and the ticked accountability box.
- [ ] In the PR body, list the oss-admin change James applies in `terraform/codecite`: Pages `build_type = "workflow"`; the `github-pages` environment limited to `main`; `docs-build` (and `Analyze (javascript-typescript)` if added) as required checks; `pages.cname` if a custom domain was chosen. The deploy job fails until the Pages source is set, so say so.
- [ ] Add no `eval` label: the site changes no indexing, search, ranking or CLI output.
- [ ] Stop at PR open.

# Docusaurus documentation site — design

codecite's documentation is Markdown read on GitHub: `README.md`,
`docs/usage.md`, `docs/design.md` and `evals/README.md`. This spec designs a
Docusaurus site that renders those files, published to GitHub Pages, without
a second copy of any of them. Issue #291 tracks the work; building the site is
a later PR that follows the plan in
`docs/superpowers/plans/2026-11-23-docusaurus-site.md`.

## Goal and audience

Two kinds of reader, both of whom today scroll a 1,200-line `usage.md` on
GitHub:

- **Agent builders** wiring codecite into an agent: the MCP tools and their
  JSON, the agent snippet, the hook recipes (`context`, `assist`), the
  citation forms and the license-class instructions an agent must follow.
- **CLI and MCP users** indexing their own repositories: manifests, `add`,
  `index`, `search`, `grep`, `def`, `read`, `refs`, `graph`, `cite check` in
  CI, release verification.

The site gives them navigation by section, a table of contents per page,
search across every page, and a CI check that internal links and anchors
hold. It adds no new prose: what it shows is what the living docs say.

Not goals: a blog, a marketing landing page, API docs for the Rust crates
(docs.rs covers those once the crates publish), or translated docs.

## Single source of truth

The living docs stay where `AGENTS.md`'s "Where things live" table puts them,
and they stay the files agents and GitHub readers use. The site reads them in
place:

- One `@docusaurus/plugin-content-docs` instance (through the classic preset)
  whose `path` is the repository root (`'..'` from `website/`), restricted by
  `include` to the published files:

  ```ts
  docs: {
    path: '..',
    include: ['README.md', 'docs/usage.md', 'docs/design.md', 'evals/README.md'],
    exclude: ['**/node_modules/**', 'website/**', 'docs/superpowers/**'],
    routeBasePath: '/',
    sidebarPath: './sidebars.ts',
    editUrl: 'https://github.com/jamescrowley321/codecite/edit/main/',
  }
  ```

  The repository root as the content root keeps every relative link the
  files already use working unchanged: the README's `docs/usage.md` and
  `docs/design.md` resolve to those pages, and `README.md` becomes the site's
  index page by Docusaurus's README-as-index convention.
- `markdown.format: 'detect'` parses `.md` files as CommonMark, not MDX. The
  docs are full of `<corpus>`, `<citation>` and `{"results": [...]}` in prose
  and tables, which MDX would read as JSX and expressions and fail on. With
  `detect`, no living doc changes to suit the site.
- `markdown.mermaid: true` and `@docusaurus/theme-mermaid` render the seven
  Mermaid diagrams (`README.md` one, `usage.md` one, `design.md` five) as
  GitHub does.
- Links to repository files that are not published pages (`LICENSE`,
  `NOTICE`, and any later link into `crates/` or `evals/`) are rewritten to
  `https://github.com/jamescrowley321/codecite/blob/main/<path>` by a small
  local remark plugin in `website/src/remark/repo-links.ts`. The file on
  GitHub keeps its relative link; only the rendered site rewrites it.
- `design.md`'s front matter (`last_verified`) is ignored by Docusaurus; page
  titles come from each file's first `#` heading. The sidebar labels live in
  `website/sidebars.ts`, not in the docs.

Nothing is copied, generated into the repo, or committed twice. A change to a
living doc changes the site on the next deploy from `main`.

### What is published, and what is not

| Source | On the site | Why |
| --- | --- | --- |
| `README.md` | Overview, the index page `/` | The entry point people already land on |
| `docs/usage.md` | Using codecite | The how-to; most readers' page |
| `docs/design.md` | Design | Why and what; see the open question on whether it is public-facing |
| `evals/README.md` | Evaluation | How the A/B gate and benchmarks work, for readers judging codecite's claims |
| `docs/superpowers/specs/`, `docs/superpowers/plans/` | No; the Design page's sidebar links to the folder on GitHub | Frozen once merged, written for one PR each, and superseded by `design.md` (`AGENTS.md`); 70-plus pages that would bury the living docs and read as current |
| `docs/cloud-sessions.md`, `AGENTS.md`, `ralph/README.md` | No | Instructions for agents working on codecite, not for its users |
| `experiments/**/README.md` | No | Raw spike notes, kept for re-running |

`usage.md` stays one page. Splitting it into a page per command would read
better on the site but means editing the living doc to suit the site; if that
is wanted later, it is a change to `docs/` made on its own merits, and the
site follows it.

## Approaches considered

**A. One docs instance rooted at the repository, `include` list
(recommended).** Reads the files in place, keeps relative links working,
needs no build step beyond `docusaurus build`. The cost: Docusaurus scans
from the repo root, so `include` and `exclude` must stay tight, and the doc
IDs carry their directory (`docs/usage`), which only the sidebar file sees.

**B. Docs instance at `../docs`, plus a sync step for the README and the
evals README.** `docs/` is the natural root, but `README.md` and
`evals/README.md` live outside it, so a `prebuild` script copies them into a
gitignored `website/generated/` folder (or a second docs instance serves
each). Links between the README and `docs/` then cross instances and break
unless rewritten. More moving parts for the same output.

**C. Move or copy the docs into `website/docs/`.** The Docusaurus default.
Rejected: it duplicates the living docs or moves them away from where
`AGENTS.md`, agents and GitHub readers expect them, against the
single-source rule.

**Generators other than Docusaurus.** mdBook (Rust, one binary, no npm) and
MkDocs Material (Python, which `evals/` already uses through uv) would add a
smaller dependency surface than Docusaurus's npm tree, which runs to roughly
a thousand transitive packages that Dependabot and the scanners then track.
Docusaurus is kept because it is the generator asked for and it brings
built-in versioning, Mermaid, a strict broken-link and broken-anchor check,
and a mature local-search plugin. The npm surface is contained as described
under Security gates.

## Where it lives and how it builds

```text
website/
  package.json            # docusaurus, preset-classic, theme-mermaid, local search; "engines"
  package-lock.json       # the only npm lockfile for the site
  .nvmrc                  # the Node LTS the workflows use
  docusaurus.config.ts
  sidebars.ts
  src/remark/repo-links.ts
  src/css/custom.css
  static/img/             # logo and favicon, if any (open question)
```

- `website/build/`, `website/.docusaurus/` and `website/node_modules/` are
  gitignored.
- `npm ci --ignore-scripts` installs, so no dependency's install script runs
  in CI; the plan's first task checks that the build works without them.
- `npm run build` runs `docusaurus build`; `npm run start` serves it locally.
- Site URL `https://jamescrowley321.github.io`, `baseUrl: '/codecite/'`,
  unless a custom domain is chosen (open question).
- `onBrokenLinks: 'throw'`, `onBrokenAnchors: 'throw'` and
  `markdown.hooks.onBrokenMarkdownLinks: 'throw'`, so any broken internal
  link or `#anchor` fails the build. `usage.md` links within itself by
  anchor (`#diff-a-specs-versions`); Docusaurus slugs headings with
  `github-slugger`, as GitHub does, so those anchors match.

## Hosting: GitHub Pages from Actions

Two jobs in one workflow, `.github/workflows/docs.yml`:

- **`docs-build`** on `pull_request` and on `push` to `main`, when
  `website/**`, `README.md`, `docs/*.md`, `evals/README.md` or the workflow
  changes. `permissions: contents: read`. Checks out with
  `persist-credentials: false`, sets up Node from `website/.nvmrc`, runs
  `npm ci --ignore-scripts` and `npm run build`. On `main` it uploads
  `website/build` with `actions/upload-pages-artifact`.
- **`docs-deploy`** on `push` to `main` only, `needs: docs-build`,
  `environment: github-pages`, with job-level
  `permissions: pages: write, id-token: write` and nothing else; it runs
  `actions/deploy-pages`. Workflow-level `permissions: {}`.

Every action is pinned by full commit SHA with its version in a comment, as
the existing workflows are, and Dependabot's `github-actions` entry already
bumps them. For zizmor:

- no `${{ }}` expansion inside `run:` blocks (template injection);
- `persist-credentials: false` on checkout (artipacked);
- no `actions/setup-node` cache in the workflow that deploys, since zizmor's
  cache-poisoning audit flags a cache restored into a job whose output is
  published; the install is small enough to run uncached;
- `concurrency: { group: pages, cancel-in-progress: false }` so two deploys
  never race and a running one is never cut off;
- no `pull_request_target`, no secrets: Pages deploys through OIDC.

A required check with a `paths` filter never reports on a PR that doesn't
touch those paths, which blocks the merge. So `docs-build` is either left
out of the required checks, or the workflow runs on every PR and a first step
exits early when no relevant path changed. The plan takes the second, so the
link check can be required.

**Follow-up in oss-admin (`terraform/codecite`):** set the repository's Pages
source to GitHub Actions (`pages { build_type = "workflow" }` on the
`github_repository` resource), add the `github-pages` environment with a
deployment branch policy of `main` only, add `docs-build` to the required
checks if it is to gate, and set `pages.cname` if a custom domain is chosen.
None of this is changed in the GitHub UI. The site cannot deploy until that
change is applied, so the build PR lands the workflow with the deploy job
and James applies the Terraform before or right after merging.

## Versioning

Docusaurus versioning snapshots `docs` into `versioned_docs/version-X.Y.Z/`
on each `docs:version` call: a committed copy of every page per release.
That is the duplication the single-source rule forbids, and codecite has no
release yet, so the site starts unversioned: it shows `main`, which is what
the code on `main` does. Once releases exist, two options keep one source:
build the site from the release tag's `docs/` in a separate deploy (a
version dropdown linking to per-tag builds), or accept Docusaurus's snapshots
for released versions only, treating each snapshot as frozen the way specs
are. That choice is James's (open question).

## Search

`@easyops-cn/docusaurus-search-local` builds a search index at build time
and searches in the browser: no external service, no account, no crawler,
no telemetry, which fits codecite's own "fully local" goal. Algolia DocSearch
is the usual alternative; it needs an application, runs a crawler on
Algolia's side and sends every query to Algolia, so it is not recommended.
The local plugin is a third-party package, so it is pinned and scanned like
the rest.

## CI check for broken links

- Internal links and anchors: the build itself, with the three `throw`
  settings above. `docs-build` failing is the check.
- External links are not gated: a third-party site being down would turn
  unrelated PRs red. A scheduled, non-required job running `lychee` over the
  built site can report them later if wanted; it is not part of this work.

## Security gates for npm dependencies

- **Lockfile:** one `website/package-lock.json`, committed; CI installs with
  `npm ci`, so it installs exactly what the lockfile says, or fails.
- **dependency-review:** already runs on every PR and diffs every lockfile
  it finds, so a PR adding a high-severity vulnerability in the site's tree
  fails it with no workflow change. `license-check` stays off, as for
  `evals/`: the site is not distributed as software. If James wants the
  site's bundled JavaScript license-checked, that is a separate step.
- **OSV-Scanner:** the reusable workflows scan the repository recursively,
  so `website/package-lock.json` is picked up; its header comment, which
  names only `Cargo.lock` and the eval fixture's lockfile, is updated.
- **Dependabot:** a new `npm` entry for `/website`, weekly, 7-day cooldown,
  `commit-message.prefix: "fix(deps)"`, its minor and patch updates grouped
  as `website-minor-patch`, majors one PR each, `open-pull-requests-limit: 5`.
  Docusaurus releases its packages together, so a group keeps them in step.
- **CodeQL:** the matrix analyses `actions` and `rust`. The site adds a few
  TypeScript files (config, sidebar, remark plugin); adding
  `javascript-typescript` to the matrix adds a new check,
  `Analyze (javascript-typescript)`, which oss-admin would have to require
  to gate. Recommended, since the remark plugin rewrites links.
- **cargo-deny** is unaffected; it covers only the Rust workspace.
- **gitleaks** already scans every file.
- `npm ci --ignore-scripts` keeps install scripts from running in CI.

## Changes to living docs in the build PR

- `AGENTS.md`'s "Where things live" table gains a row: the documentation
  site, `website/`, rendering `README.md`, `docs/usage.md`, `docs/design.md`
  and `evals/README.md` in place.
- `README.md`'s Documentation list gains the site's URL.
- `docs/design.md`'s supply-chain bullets name the site's npm tree and the
  Pages deploy beside the existing gates.

## Open questions for James

1. **Custom domain.** `jamescrowley321.github.io/codecite/`, or a domain of
   your own (which needs a DNS record and `pages.cname` in oss-admin)?
2. **Versioned docs.** Unversioned (`main` only) until the first release, as
   recommended, and then per-tag builds or Docusaurus snapshots?
3. **Theme and branding.** The classic theme with its defaults, or a logo,
   colours and favicon? There is no codecite logo today.
4. **Is `design.md` public-facing?** It is in a public repository already,
   but a site puts it in the navigation beside the usage guide. It names
   first users and the history benchmark's repositories, holds the
   evaluation plan's per-task numbers and the decisions table. Publish it as
   is, publish it under a "Project" heading apart from the user docs, or
   leave it off the site and link to it on GitHub?
5. **`evals/README.md` on the site** under Evaluation, as recommended, or
   left on GitHub?
6. **Gating.** Make `docs-build` a required check in oss-admin (recommended,
   so a broken link can't merge), and add `Analyze (javascript-typescript)`
   to CodeQL's required checks?

# Security Policy

codecite indexes code and specifications and serves them to AI agents over a
CLI and an MCP server. It fetches documents named in manifests and caches them
locally, so a flaw in fetching, caching or serving can expose what an agent
reads or what lands on disk. Reports are taken seriously and triaged promptly.

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report privately through GitHub's
**[private vulnerability reporting](https://github.com/jamescrowley321/codecite/security/advisories/new)**
(the repository's **Security** tab, then **Report a vulnerability**). This opens
a private advisory, visible only to you and the maintainer, where a fix can be
coordinated.

Please include as much of the following as you can:

- The codecite version or commit.
- A description of the issue and its security impact.
- A minimal reproduction: a manifest, a corpus, a command or an MCP request
  that shows the problem.

## What to expect

- **Acknowledgement** within 3 business days.
- An initial assessment (severity, affected versions, likely fix) within 10
  business days.
- Coordinated disclosure: the fix is prepared privately and released, and only
  then is the advisory published, crediting the reporter unless you prefer to
  stay anonymous.

## Supported versions

Security fixes go into the latest release on crates.io and the `main` branch.
Older releases are not patched; upgrade to the current release to get fixes.

## Scope

In scope:

- The `codecite` binary and its CLI.
- The MCP server.
- Manifest fetching and caching.

Out of scope:

- The contents of indexed third-party corpora. codecite indexes and cites them
  but does not vouch for them.
- The `evals/` harness, which runs only on the maintainer's machine and is
  never shipped.

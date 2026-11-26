import type {ReactNode} from 'react';
import Link from '@docusaurus/Link';
import Layout from '@theme/Layout';
import CodeBlock from '@theme/CodeBlock';

import styles from './index.module.css';

const GITHUB = 'https://github.com/jamescrowley321/codecite';

// The search from the README's quick start, verbatim: the command, its
// summary line and its three hits. Each hit is split only to color it.
const QUERY = 'codecite search demo "detect a repository license" --limit 3';
const SUMMARY = '3 results · adapt · indexed 2026-11-26T16:03:24Z';
const HITS: [cite: string, kind: string, first: string][] = [
  [
    'codecite@7728a43d1da7:crates/codecite-index/src/manifest.rs#L219-L227',
    'function license',
    '/// The license as the manifest writes it, when it gives one.',
  ],
  [
    'codecite@7728a43d1da7:crates/codecite-index/src/license.rs#L126-L159',
    'function detect_license',
    "/// Matches the license files at `dir`'s top level. Only regular files",
  ],
  [
    'codecite@7728a43d1da7:crates/codecite-index/src/license.rs#L274-L282',
    'function detect',
    'fn detect(files: &[(&str, &str)]) -> DetectedLicense {',
  ],
];

// The parts of one citation, in order, with what each one pins.
const CITATION: [part: string, label: string][] = [
  ['codecite', 'repository'],
  ['@7728a43d1da7', 'pinned commit'],
  [':crates/codecite-index/src/license.rs', 'file'],
  ['#L126-L159', 'lines'],
];

const POINTS: {title: string; body: ReactNode}[] = [
  {
    title: 'Code and prose in one ranking',
    body: (
      <>
        One BM25 ranking over code, docs and specs. Identifiers match however
        they are written: <code>validate token</code> finds{' '}
        <code>validateToken</code>, <code>validate_token</code> and{' '}
        <code>validate-token</code>.
      </>
    ),
  },
  {
    title: 'Built for agents',
    body: (
      <>
        An MCP server whose tools return the same JSON as the CLI, a Claude
        Code plugin, and hooks that put codecite&rsquo;s answers next to an
        agent&rsquo;s own grep.
      </>
    ),
  },
  {
    title: 'Pinned and local',
    body: (
      <>
        Git sources are pinned at a commit and documents by sha256. One SQLite
        file per corpus, no telemetry, and after the first run indexing works
        offline.
      </>
    ),
  },
  {
    title: 'Measured, not assumed',
    body: (
      <>
        An A/B eval runs the same agent on the same task with and without
        codecite. A change that makes agents pass less often or run much
        slower loses the gate.
      </>
    ),
  },
];

const DOCS: {to: string; title: string; body: string}[] = [
  {
    to: '/docs',
    title: 'Overview and quick start',
    body: 'Install, index a first repository and connect an agent.',
  },
  {
    to: '/docs/usage',
    title: 'Using codecite',
    body: 'Manifests, indexing, search, citation forms, hooks, the plugin and connecting an agent.',
  },
  {
    to: '/docs/evals',
    title: 'Evaluation',
    body: 'The A/B agent eval and the benchmarks that decide whether codecite earns its place.',
  },
  {
    to: '/docs/design',
    title: 'Design',
    body: 'Goals, architecture, data model, ranking, licensing and security.',
  },
];

function Terminal(): ReactNode {
  return (
    <figure className={styles.terminal} aria-label="An example codecite search">
      <pre className={styles.terminalBody} tabIndex={0}>
        <code>
          <span className={styles.line}>
            <span className={styles.prompt} aria-hidden="true">
              ${' '}
            </span>
            {QUERY}
          </span>
          <span className={`${styles.line} ${styles.dim}`}>{SUMMARY}</span>
          {HITS.map(([cite, kind, first]) => (
            <span key={cite} className={styles.line}>
              <span className={styles.cite}>{cite}</span>
              <span className={styles.dim}> · </span>
              <span className={styles.kind}>{kind}</span>
              <span className={styles.dim}> · </span>
              {first}
            </span>
          ))}
        </code>
      </pre>
    </figure>
  );
}

export default function Home(): ReactNode {
  return (
    <Layout
      title="Cited code search for AI agents"
      description="codecite indexes your repositories and their docs into one SQLite file and answers AI coding agents with ranked results, each cited to the exact lines it came from.">
      <main className={styles.page}>
        <header className={styles.hero}>
          <div className={styles.wrap}>
            <h1 className={styles.title}>
              Ranked, cited answers about your code and docs, for AI coding
              agents.
            </h1>
            <p className={styles.lede}>
              codecite is a single Rust binary that indexes your repositories
              and the documents kept with them into one SQLite file per
              corpus. Agents query it over a CLI or an MCP server, and every
              result says exactly where it came from.
            </p>
            <div className={styles.actions}>
              <Link className="button button--primary button--lg" to="/docs">
                Get started
              </Link>
              <Link className="button button--secondary button--lg" href={GITHUB}>
                GitHub
              </Link>
            </div>
          </div>
          <div className={`${styles.wrap} ${styles.terminalWrap}`}>
            <Terminal />
          </div>
        </header>

        <section className={styles.section} aria-labelledby="citations">
          <div className={styles.wrap}>
            <h2 id="citations" className={styles.h2}>
              Every result points at exact lines
            </h2>
            <p className={styles.citation} aria-label={CITATION.map(([p]) => p).join('')}>
              {CITATION.map(([part, label]) => (
                <span key={part} className={styles.part} aria-hidden="true">
                  <span className={styles.partText}>{part}</span>
                  <span className={styles.partLabel}>{label}</span>
                </span>
              ))}
            </p>
            <p className={styles.prose}>
              <code>codecite read</code> turns a citation back into the lines
              it names, exactly as they were at that commit, so an agent
              builds on real code instead of a paraphrase. A citation that
              doesn&rsquo;t resolve to exactly one span is an error listing
              the candidates, never a guess.
            </p>
          </div>
        </section>

        <section className={`${styles.section} ${styles.alt}`} aria-labelledby="why">
          <div className={styles.wrap}>
            <h2 id="why" className={styles.h2}>
              Why agents reach for it
            </h2>
            <div className={styles.points}>
              {POINTS.map(({title, body}) => (
                <div key={title} className={styles.point}>
                  <h3 className={styles.h3}>{title}</h3>
                  <p>{body}</p>
                </div>
              ))}
            </div>
          </div>
        </section>

        <section className={styles.section} aria-labelledby="install">
          <div className={`${styles.wrap} ${styles.install}`}>
            <div>
              <h2 id="install" className={styles.h2}>
                Install
              </h2>
              <p className={styles.prose}>
                Build the binary from the repository. No release has been cut
                yet. It needs Rust 1.99.1 or newer and a C compiler.
              </p>
            </div>
            <div className={styles.installSteps}>
              <CodeBlock language="sh">
                cargo install --git https://github.com/jamescrowley321/codecite codecite --locked
              </CodeBlock>
              <p className={styles.prose}>Then register the MCP server with your agent, for example Claude Code:</p>
              <CodeBlock language="sh">claude mcp add codecite -- codecite mcp</CodeBlock>
              <p className={styles.prose}>
                The <Link to="/docs">quick start</Link> walks through adding,
                indexing and searching a first corpus.
              </p>
            </div>
          </div>
        </section>

        <section className={`${styles.section} ${styles.alt}`} aria-labelledby="docs">
          <div className={styles.wrap}>
            <h2 id="docs" className={styles.h2}>
              Look something up
            </h2>
            <ul className={styles.docs}>
              {DOCS.map(({to, title, body}) => (
                <li key={to}>
                  <Link to={to} className={styles.doc}>
                    <span className={styles.docTitle}>{title}</span>
                    <span className={styles.docBody}>{body}</span>
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        </section>
      </main>
    </Layout>
  );
}

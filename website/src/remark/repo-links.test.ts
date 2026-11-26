import {test, before, after} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import repoLinks from './repo-links.ts';

const BLOB = 'https://github.com/jamescrowley321/codecite/blob/main';
const TREE = 'https://github.com/jamescrowley321/codecite/tree/main';

let root: string;

before(() => {
  // A stand-in for the repository root.
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'repo-links-'));
  for (const file of [
    'LICENSE',
    'NOTICE',
    'AGENTS.md',
    'README.md',
    'docs/usage.md',
    'crates/codecite/src/main.rs',
  ]) {
    fs.mkdirSync(path.dirname(path.join(root, file)), {recursive: true});
    fs.writeFileSync(path.join(root, file), '');
  }
});

after(() => {
  fs.rmSync(root, {recursive: true, force: true});
});

type Node = {type: string; url?: string; children?: Node[]};

// Runs the plugin over one link (or definition) in a doc at `docPath`
// (relative to the fixture root) and returns the resulting URL.
function rewrite(url: string, docPath = 'README.md', type = 'link'): string {
  const node: Node =
    type === 'link'
      ? {type, url, children: [{type: 'text'}]}
      : {type, url};
  const tree: Node = {type: 'root', children: [{type: 'paragraph', children: [node]}]};
  repoLinks({repoRoot: root})(tree, {path: path.join(root, docPath)});
  return node.url as string;
}

test('a link to an existing non-Markdown file points at GitHub', () => {
  assert.equal(rewrite('LICENSE'), `${BLOB}/LICENSE`);
  assert.equal(rewrite('NOTICE'), `${BLOB}/NOTICE`);
  assert.equal(rewrite('./NOTICE'), `${BLOB}/NOTICE`);
  assert.equal(
    rewrite('crates/codecite/src/main.rs'),
    `${BLOB}/crates/codecite/src/main.rs`,
  );
});

test('the path resolves from the doc directory', () => {
  assert.equal(rewrite('../LICENSE', 'docs/usage.md'), `${BLOB}/LICENSE`);
  assert.equal(
    rewrite('../crates/codecite/src/main.rs', 'docs/usage.md'),
    `${BLOB}/crates/codecite/src/main.rs`,
  );
});

test('a link to a directory points at the GitHub tree view', () => {
  assert.equal(rewrite('crates'), `${TREE}/crates`);
  assert.equal(rewrite('crates/'), `${TREE}/crates`);
});

test('the fragment and query are kept, and stripped for the lookup', () => {
  assert.equal(rewrite('LICENSE#L1'), `${BLOB}/LICENSE#L1`);
  assert.equal(rewrite('LICENSE?plain=1'), `${BLOB}/LICENSE?plain=1`);
  assert.equal(rewrite('LICENSE?plain=1#L3'), `${BLOB}/LICENSE?plain=1#L3`);
});

test('a link to a missing file is left for the build to report', () => {
  assert.equal(rewrite('docs/missing.md'), 'docs/missing.md');
  assert.equal(rewrite('crates/gone.rs'), 'crates/gone.rs');
  assert.equal(rewrite('LICENCE'), 'LICENCE');
});

test('a link to Markdown, published or not, is left untouched', () => {
  assert.equal(rewrite('AGENTS.md'), 'AGENTS.md');
  assert.equal(rewrite('docs/usage.md'), 'docs/usage.md');
  assert.equal(rewrite('docs/usage.md#search'), 'docs/usage.md#search');
  assert.equal(rewrite('../AGENTS.md', 'docs/usage.md'), '../AGENTS.md');
});

test('anchors, absolute URLs and site paths are left untouched', () => {
  for (const url of [
    '#search',
    'https://example.com/LICENSE',
    '//example.com/LICENSE',
    'mailto:someone@example.com',
    '/LICENSE',
    '',
  ]) {
    assert.equal(rewrite(url), url);
  }
});

test('a path that climbs above the repository root is left untouched', () => {
  assert.equal(rewrite('../LICENSE'), '../LICENSE');
  assert.equal(rewrite('../../LICENSE', 'docs/usage.md'), '../../LICENSE');
});

test('the repository root itself is left untouched', () => {
  assert.equal(rewrite('.'), '.');
  assert.equal(rewrite('..', 'docs/usage.md'), '..');
});

test('a malformed percent-encoding is left untouched', () => {
  assert.equal(rewrite('LICENSE%'), 'LICENSE%');
});

test('reference-style links are rewritten through their definition', () => {
  assert.equal(rewrite('LICENSE', 'README.md', 'definition'), `${BLOB}/LICENSE`);
  assert.equal(
    rewrite('crates/gone.rs', 'README.md', 'definition'),
    'crates/gone.rs',
  );
});

test('a file without a path is left alone', () => {
  const node: Node = {type: 'link', url: 'LICENSE', children: []};
  repoLinks({repoRoot: root})({type: 'root', children: [node]}, {});
  assert.equal(node.url, 'LICENSE');
});

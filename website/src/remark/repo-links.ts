// A remark plugin for the documentation site. The site renders the living
// docs in place, so a relative link to a repository file that is not a
// published page (LICENSE, NOTICE, a source file) would 404 on the site.
// This plugin points such a link at the file on GitHub. It rewrites a link
// only when it is relative, resolves inside the repository, names a file or
// directory that exists, and is not Markdown. Every other link is left for
// Docusaurus's broken-link checks to resolve or fail, so a mistyped path or a
// link to an unpublished doc fails the build instead of escaping to GitHub.

import fs from 'node:fs';
import path from 'node:path';

export type RepoLinksOptions = {
  // Absolute path of the repository root.
  repoRoot: string;
  // GitHub repository URL, without a trailing slash.
  repoUrl?: string;
  // Branch the links point at.
  branch?: string;
};

type MdNode = {
  type: string;
  url?: string;
  children?: MdNode[];
};

type VFileLike = {path?: string};

const DEFAULT_REPO_URL = 'https://github.com/jamescrowley321/codecite';
const SCHEME = /^[a-zA-Z][a-zA-Z\d+.-]*:/;

// Returns the GitHub URL for `url` found in the doc at `docPath`, or
// undefined when the link is to be left as it is.
export function githubUrlFor(
  url: string,
  docPath: string,
  options: RepoLinksOptions,
): string | undefined {
  if (url === '' || SCHEME.test(url) || url.startsWith('/') || url.startsWith('#')) {
    return undefined;
  }

  const hashAt = url.indexOf('#');
  const fragment = hashAt === -1 ? '' : url.slice(hashAt);
  const beforeHash = hashAt === -1 ? url : url.slice(0, hashAt);
  const queryAt = beforeHash.indexOf('?');
  const query = queryAt === -1 ? '' : beforeHash.slice(queryAt);
  const rawPath = queryAt === -1 ? beforeHash : beforeHash.slice(0, queryAt);
  if (rawPath === '') {
    return undefined;
  }

  let decoded: string;
  try {
    decoded = decodeURIComponent(rawPath);
  } catch {
    return undefined;
  }

  const root = path.resolve(options.repoRoot);
  const target = path.resolve(path.dirname(docPath), decoded);
  const relative = path.relative(root, target);
  if (
    relative === '' ||
    relative === '..' ||
    relative.startsWith(`..${path.sep}`) ||
    path.isAbsolute(relative)
  ) {
    return undefined;
  }
  if (/\.mdx?$/i.test(relative)) {
    return undefined;
  }

  let stat: fs.Stats;
  try {
    stat = fs.statSync(target);
  } catch {
    return undefined;
  }

  const repoUrl = options.repoUrl ?? DEFAULT_REPO_URL;
  const branch = options.branch ?? 'main';
  const view = stat.isDirectory() ? 'tree' : 'blob';
  const urlPath = relative.split(path.sep).map(encodeURIComponent).join('/');
  return `${repoUrl}/${view}/${branch}/${urlPath}${query}${fragment}`;
}

export default function repoLinks(options: RepoLinksOptions) {
  return (tree: MdNode, file: VFileLike): void => {
    const docPath = file.path;
    if (!docPath) {
      return;
    }
    const visit = (node: MdNode): void => {
      if ((node.type === 'link' || node.type === 'definition') && typeof node.url === 'string') {
        const rewritten = githubUrlFor(node.url, docPath, options);
        if (rewritten !== undefined) {
          node.url = rewritten;
        }
      }
      for (const child of node.children ?? []) {
        visit(child);
      }
    };
    visit(tree);
  };
}

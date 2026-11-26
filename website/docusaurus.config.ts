import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';
import path from 'node:path';

import repoLinks from './src/remark/repo-links.ts';

// Runs in Node.js at build time.

const config: Config = {
  title: 'codecite',
  tagline: 'Cited code and spec search for AI agents',

  url: 'https://jamescrowley321.github.io',
  baseUrl: '/codecite/',
  trailingSlash: false,
  organizationName: 'jamescrowley321',
  projectName: 'codecite',

  onBrokenLinks: 'throw',
  onBrokenAnchors: 'throw',

  // CommonMark, not MDX, for .md files: the docs use `<corpus>` and
  // `{"results": [...]}` in prose, which MDX would parse as JSX.
  markdown: {
    format: 'detect',
    mermaid: true,
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },

  themes: [
    '@docusaurus/theme-mermaid',
    [
      // Search index built at build time and searched in the browser: no
      // external service.
      '@easyops-cn/docusaurus-search-local',
      {
        indexDocs: true,
        indexBlog: false,
        indexPages: false,
        docsRouteBasePath: '/',
        hashed: true,
      },
    ],
  ],

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      {
        // The site reads the living docs in place: the docs plugin is rooted
        // at the repository, so the relative links the files use on GitHub
        // keep working, and `include` names the only files published. Specs,
        // plans, docs/cloud-sessions.md and AGENTS.md are not published.
        docs: {
          path: '..',
          include: [
            'README.md',
            'docs/usage.md',
            'docs/design.md',
            'evals/README.md',
          ],
          exclude: ['**/node_modules/**', 'website/**', 'docs/superpowers/**'],
          routeBasePath: '/',
          sidebarPath: './sidebars.ts',
          editUrl: 'https://github.com/jamescrowley321/codecite/edit/main/',
          // Before Docusaurus resolves links, so it never sees a rewritten one.
          beforeDefaultRemarkPlugins: [
            [repoLinks, {repoRoot: path.resolve(__dirname, '..')}],
          ],
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    colorMode: {
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'codecite',
      items: [
        {
          href: 'https://github.com/jamescrowley321/codecite',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      copyright:
        'codecite is licensed under the <a href="https://github.com/jamescrowley321/codecite/blob/main/LICENSE">Apache License 2.0</a>.',
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
    },
  } satisfies Preset.ThemeConfig,
};

export default config;

import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';
import path from 'node:path';

import repoLinks from './src/remark/repo-links.ts';

// Runs in Node.js at build time.

const GITHUB = 'https://github.com/jamescrowley321/codecite';

// Each published file's URL under /docs, by its path in the repository.
const DOC_SLUGS: Record<string, string> = {
  'README.md': '/',
  'docs/usage.md': '/usage',
  'docs/design.md': '/design',
  'evals/README.md': '/evals',
};

const config: Config = {
  title: 'codecite',
  tagline: 'Ranked, cited answers about your code and docs, for AI coding agents',
  favicon: 'img/favicon.svg',

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
    // The docs live under /docs, and the landing page at /. The published
    // files carry no front matter for the site, so their URLs are set here:
    // /docs, /docs/usage, /docs/design and /docs/evals.
    parseFrontMatter: async (params) => {
      const result = await params.defaultParseFrontMatter(params);
      const relative = path
        .relative(path.resolve(__dirname, '..'), params.filePath)
        .split(path.sep)
        .join('/');
      const slug = DOC_SLUGS[relative];
      if (slug !== undefined && result.frontMatter.slug === undefined) {
        result.frontMatter.slug = slug;
      }
      return result;
    },
  },

  headTags: [
    {tagName: 'link', attributes: {rel: 'icon', href: '/codecite/favicon.ico', sizes: '48x48'}},
    {
      tagName: 'link',
      attributes: {rel: 'apple-touch-icon', href: '/codecite/img/apple-touch-icon.png'},
    },
    {tagName: 'link', attributes: {rel: 'preconnect', href: 'https://fonts.googleapis.com'}},
    {
      tagName: 'link',
      attributes: {rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: 'anonymous'},
    },
  ],
  stylesheets: [
    'https://fonts.googleapis.com/css2?family=Bricolage+Grotesque:opsz,wght@12..96,400..700&family=Geist+Mono:wght@400..600&display=swap',
  ],

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
        docsRouteBasePath: 'docs',
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
          routeBasePath: 'docs',
          sidebarPath: './sidebars.ts',
          editUrl: `${GITHUB}/edit/main/`,
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
    image: 'img/social-card.png',
    metadata: [
      {
        name: 'description',
        content:
          'codecite indexes your repositories and their docs into one SQLite file and answers AI coding agents with ranked results, each cited to the exact lines it came from.',
      },
    ],
    colorMode: {
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'codecite',
      logo: {
        alt: 'codecite',
        src: 'img/logo.svg',
        srcDark: 'img/logo-dark.svg',
        width: 28,
        height: 28,
      },
      items: [
        // Plain links, not `type: 'doc'`: the docs share one sidebar, and a
        // doc item lights up for every page in its sidebar.
        {to: '/docs', label: 'Docs', position: 'left', activeBaseRegex: '/docs/?$'},
        {to: '/docs/usage', label: 'Usage', position: 'left', activeBaseRegex: '/docs/usage/?$'},
        {to: '/docs/evals', label: 'Evaluation', position: 'left', activeBaseRegex: '/docs/evals/?$'},
        {to: '/docs/design', label: 'Design', position: 'left', activeBaseRegex: '/docs/design/?$'},
        {href: GITHUB, label: 'GitHub', position: 'right'},
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Docs',
          items: [
            {label: 'Overview', to: '/docs'},
            {label: 'Using codecite', to: '/docs/usage'},
            {label: 'Evaluation', to: '/docs/evals'},
            {label: 'Design', to: '/docs/design'},
          ],
        },
        {
          title: 'Project',
          items: [
            {label: 'GitHub', href: GITHUB},
            {label: 'Issues', href: `${GITHUB}/issues`},
            {label: 'Milestones', href: `${GITHUB}/milestones`},
            {label: 'Security policy', href: `${GITHUB}/blob/main/SECURITY.md`},
          ],
        },
        {
          title: 'License',
          items: [
            {label: 'Apache License 2.0', href: `${GITHUB}/blob/main/LICENSE`},
            {label: 'NOTICE', href: `${GITHUB}/blob/main/NOTICE`},
          ],
        },
      ],
      copyright: 'codecite is licensed under the Apache License 2.0.',
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
    },
  } satisfies Preset.ThemeConfig,
};

export default config;

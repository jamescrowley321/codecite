import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

// Runs in Node.js at build time. Labels live here, not in the docs: each
// page's title is its file's first heading.

const sidebars: SidebarsConfig = {
  docs: [
    {type: 'doc', id: 'README', label: 'Overview'},
    {type: 'doc', id: 'docs/usage', label: 'Using codecite'},
    {type: 'doc', id: 'evals/README', label: 'Evaluation'},
    {
      type: 'category',
      label: 'Project',
      collapsed: false,
      items: [
        {type: 'doc', id: 'docs/design', label: 'Design'},
        {
          type: 'link',
          label: 'Design specs and plans',
          href: 'https://github.com/jamescrowley321/codecite/tree/main/docs/superpowers',
        },
      ],
    },
  ],
};

export default sidebars;

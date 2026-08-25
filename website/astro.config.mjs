// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import starlightLinksValidator from 'starlight-links-validator';

const BOOK_URL = 'https://rickymillar.github.io/sysmlv2-book/';

// GitHub Pages project site: https://rickymillar.github.io/sysml-rs/
export default defineConfig({
  site: 'https://rickymillar.github.io',
  base: '/sysml-rs',
  trailingSlash: 'ignore',
  integrations: [
    starlight({
      title: 'sysml-rs',
      description:
        'Documentation portal for sysml-rs, a pre-alpha Rust implementation of OMG SysML v2.',
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/RickyMillar/sysml-rs',
        },
      ],
      editLink: {
        baseUrl: 'https://github.com/RickyMillar/sysml-rs/edit/main/website/',
      },
      customCss: ['./src/styles/custom.css'],
      routeMiddleware: './src/routeData.ts',
      components: {
        PageTitle: './src/components/PageTitle.astro',
        Footer: './src/components/Footer.astro',
      },
      plugins: [
        starlightLinksValidator({
          errorOnRelativeLinks: true,
          errorOnInvalidHashes: true,
        }),
      ],
      sidebar: [
        {
          label: 'Start Here',
          items: [
            { slug: 'start-here/what-is-sysml-rs' },
            { slug: 'start-here/install' },
            { slug: 'start-here/first-model' },
            { slug: 'start-here/choose-your-path' },
          ],
        },
        {
          label: 'Learn SysML v2',
          items: [
            {
              label: 'The SysML v2 Book ↗',
              link: BOOK_URL,
              attrs: { target: '_blank', rel: 'noopener' },
            },
            {
              label: 'Language Reference ↗',
              link: `${BOOK_URL}language-reference/index.html`,
              attrs: { target: '_blank', rel: 'noopener' },
            },
            {
              label: 'Language pack for agents ↗',
              link: `${BOOK_URL}language-pack/`,
              attrs: { target: '_blank', rel: 'noopener' },
            },
          ],
        },
        {
          label: 'Use sysml-rs',
          items: [
            { slug: 'use/cli-workflows' },
            { slug: 'use/imports-vs-dependencies' },
            { slug: 'use/sysml-toml' },
            { slug: 'use/dependencies' },
            { slug: 'use/lock-and-cache' },
            { slug: 'use/workspaces' },
            { slug: 'use/kpar' },
            { slug: 'use/runtime' },
            { slug: 'use/views-and-diagrams' },
            { slug: 'use/editors' },
            { slug: 'use/integrations' },
            { slug: 'use/examples' },
          ],
        },
        {
          label: 'Reference',
          items: [{ slug: 'reference/known-limitations' }],
        },
        {
          label: 'About',
          items: [
            { slug: 'about/licensing' },
            {
              label: 'Contributing ↗',
              link: 'https://github.com/RickyMillar/sysml-rs/blob/main/CONTRIBUTING.md',
              attrs: { target: '_blank', rel: 'noopener' },
            },
          ],
        },
      ],
    }),
  ],
});

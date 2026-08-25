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
          // The Book is built into public/learn/ as static assets by
          // scripts/build-learn.sh; its routes are not Starlight pages.
          exclude: ['/sysml-rs/learn/**'],
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
            // Starlight prepends the site base to sidebar links, so these are
            // written base-relative (unlike in-page markdown links).
            {
              label: 'The SysML v2 Book',
              link: '/learn/',
            },
            {
              label: 'Language Reference',
              link: '/learn/language-reference/',
            },
            {
              label: 'Language pack for agents',
              link: '/learn/language-pack/manifest.json',
            },
          ],
        },
        {
          label: 'Use sysml-rs',
          items: [
            { slug: 'use/cli-workflows' },
            {
              label: 'Projects & dependencies',
              items: [
                { slug: 'use/imports-vs-dependencies' },
                { slug: 'use/sysml-toml' },
                { slug: 'use/dependencies' },
                { slug: 'use/lock-and-cache' },
                { slug: 'use/workspaces' },
                { slug: 'use/kpar' },
              ],
            },
            {
              label: 'Modelling & execution',
              items: [
                { slug: 'use/runtime' },
                { slug: 'use/views-and-diagrams' },
                { slug: 'use/examples' },
              ],
            },
            {
              label: 'Interfaces',
              items: [
                { slug: 'use/integrations' },
                { slug: 'use/editors' },
                { slug: 'use/lsp' },
                { slug: 'use/service-api' },
                { slug: 'use/mcp' },
                { slug: 'use/simulation-app' },
              ],
            },
          ],
        },
        {
          label: 'Develop sysml-rs',
          items: [{ slug: 'develop/architecture' }, { slug: 'develop/code-map' }],
        },
        {
          label: 'Reference',
          items: [
            { slug: 'reference/cli-commands' },
            { slug: 'reference/api-mcp-catalog' },
            { slug: 'reference/diagnostics' },
            { slug: 'reference/capability-matrix' },
            { slug: 'reference/sysml-toml' },
            { slug: 'reference/known-limitations' },
          ],
        },
        {
          label: 'About',
          items: [
            { slug: 'about/licensing' },
            { slug: 'about/maintenance' },
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

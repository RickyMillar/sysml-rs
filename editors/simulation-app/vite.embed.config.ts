/**
 * Vite config for the standalone diagram-embed entry.
 *
 * Builds ONLY `embed.html` (src/embed/main.tsx) into `dist-embed/` with
 * relative asset paths, so the whole directory can be copied anywhere (e.g.
 * the book's src/viewer/) and loaded in an <iframe> from a plain static file
 * server — no backend, no app shell. URL params + embedding contract:
 * src/embed/README.md.
 *
 *   npx vite build --config vite.embed.config.ts
 */
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { copyFile } from 'fs/promises';
import path from 'path';

/** Font license texts (public/fonts/ is not copied — publicDir is off) that
 *  must accompany the embedded IBM Plex / Material Symbols subsets. */
const FONT_LICENSES = ['LICENSE-IBMPlex-OFL-1.1.txt', 'LICENSE-MaterialSymbols-Apache-2.0.txt'];

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    {
      name: 'embed-font-licenses',
      async closeBundle() {
        for (const name of FONT_LICENSES) {
          await copyFile(
            path.resolve(__dirname, 'public/fonts', name),
            path.resolve(__dirname, 'dist-embed', name),
          );
        }
      },
    },
  ],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
      // The app imports elk.bundled.js — the full ~1.4 MB engine — as the
      // in-thread fallback for jsdom/vitest, where `Worker` doesn't exist. In
      // the embed the browser always has Worker and layout.ts always runs elk
      // in the real worker asset (elk-worker.min.js), so swap the bundled
      // engine for the tiny elk-api client and keep it out of the main chunk.
      'elkjs/lib/elk.bundled.js': 'elkjs/lib/elk-api.js',
    },
  },
  // Relative asset URLs: the embed must work from any mount path.
  base: './',
  // No public/ copy — the app's public dir belongs to the shell build. The
  // fonts the embed needs are declared in src/embed/embed-fonts.css and
  // emitted as hashed assets (IBM Plex from public/fonts/, plus the Material
  // Symbols subset in src/embed/fonts/).
  publicDir: false,
  build: {
    outDir: 'dist-embed',
    rollupOptions: {
      input: path.resolve(__dirname, 'embed.html'),
    },
    chunkSizeWarningLimit: 1000,
  },
});

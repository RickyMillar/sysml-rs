/**
 * Vite config for the standalone diagram-embed entry (spike).
 *
 * Builds ONLY `embed.html` (src/embed/main.tsx) into `dist-embed/` with
 * relative asset paths, so the whole directory can be copied anywhere (e.g.
 * the book's src/viewer/) and loaded in an <iframe> from a plain static file
 * server — no backend, no app shell.
 *
 *   npx vite build --config vite.embed.config.ts
 */
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import path from 'path';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  // Relative asset URLs: the embed must work from any mount path.
  base: './',
  // No public/ copy — the app's public dir (vendored fonts, icons) belongs to
  // the shell build; the embed falls back to system fonts (spike trade-off).
  publicDir: false,
  build: {
    outDir: 'dist-embed',
    rollupOptions: {
      input: path.resolve(__dirname, 'embed.html'),
    },
    chunkSizeWarningLimit: 5000,
  },
});

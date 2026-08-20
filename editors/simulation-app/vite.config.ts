import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import path from 'path';

// The Sprotty diagram package (and its sysml-layout WASM router) is gone — the
// SvgCanvas renderer lays out + routes client-side with elkjs (Bucket 3), so no
// WASM-serving plugin or sysml-layout/sysml-diagram-wasm aliases are needed.

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  build: {
    chunkSizeWarningLimit: 3000,
  },
  server: {
    port: 3010,
    proxy: {
      '/sources': 'http://localhost:8080',
      '/models': 'http://localhost:8080',
      // Sessions REST endpoints live under /api/sessions/... but the
      // WebSocket stream (`/api/sessions/<id>/events`) is routed here
      // too. WS must be explicitly enabled on a proxy entry; without
      // `ws: true`, Vite forwards the upgrade HTTP response but never
      // wires the bidirectional frame pipe — the browser opens a
      // socket that receives zero messages. We keep the old
      // string-shorthand aliases for the HTTP-only routes and add
      // `ws: true` on the `/api` entry which is the one that carries
      // the session-events stream.
      '/sessions': 'http://localhost:8080',
      '/api': {
        target: 'http://localhost:8080',
        ws: true,
        changeOrigin: true,
      },
      '/health': 'http://localhost:8080',
      '/files': 'http://localhost:8080',
      '/workspace': 'http://localhost:8080',
      '/commands': 'http://localhost:8080',
      '/lsp': {
        target: 'ws://localhost:8080',
        ws: true,
      },
    },
  },
});

import { defineConfig } from 'vite';

export default defineConfig({
  root: 'examples',
  server: {
    port: 3007,
  },
  build: {
    outDir: '../dist-examples',
    emptyOutDir: true,
  },
});

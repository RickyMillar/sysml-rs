import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import path from 'path';

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    globals: false,
    include: [
      'src/__tests__/**/*.test.{ts,tsx}',
      'src/**/__tests__/**/*.test.{ts,tsx}',
      'src/**/*.test.{ts,tsx}',
    ],
    setupFiles: ['src/test/setup.ts'],
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
});

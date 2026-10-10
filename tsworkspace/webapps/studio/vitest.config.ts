import path from 'node:path';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  resolve: {
    alias: {
      '@': path.resolve(__dirname, 'src'),
    },
  },
  test: {
    include: ['src/**/*.test.{ts,tsx}', 'server/**/*.test.mjs', 'shared/**/*.test.mjs', 'e2e/fixtures/**/*.test.mjs'],
    environment: 'jsdom',
    environmentMatchGlobs: [
      ['server/**', 'node'],
      ['shared/**', 'node'],
      ['e2e/fixtures/**', 'node'],
    ],
    coverage: {
      provider: 'v8',
      thresholds: {
        lines: 65,
        functions: 50,
        branches: 40,
        statements: 60,
      },
    },
  },
});

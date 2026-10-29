import path from 'node:path';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  server: {
    fs: { allow: [path.resolve(import.meta.dirname, '../../..')] },
  },
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, 'src'),
    },
  },
  server: {
    fs: { allow: [path.resolve(import.meta.dirname, '../../..')] },
  },
  test: {
    projects: [
      {
        extends: true,
        test: {
          name: 'dom',
          include: ['src/**/*.test.{ts,tsx}'],
          environment: 'jsdom',
        },
      },
      {
        extends: true,
        test: {
          name: 'node',
          include: ['server/**/*.test.mjs', 'shared/**/*.test.mjs', 'e2e/fixtures/**/*.test.mjs'],
          environment: 'node',
        },
      },
    ],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      thresholds: {
        lines: 65,
        functions: 50,
        branches: 40,
        statements: 60,
      },
    },
  },
});

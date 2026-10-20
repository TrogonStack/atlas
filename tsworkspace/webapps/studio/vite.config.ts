import path from 'node:path';
import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

export default defineConfig(({ command }) => ({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, 'src'),
    },
  },
  // Strip `console.*` and `debugger` from production bundles only. The dev
  // server (`vite dev`) keeps both so authors still see logs in iteration.
  build:
    command === 'build'
      ? {
          // three.js gets a chunk of its own that only the 3D tabs load. It is
          // larger than the default warning size, so the limit sits just above it.
          chunkSizeWarningLimit: 600,
          rolldownOptions: {
            output: {
              minify: { compress: { dropConsole: true, dropDebugger: true } },
              codeSplitting: { groups: [{ name: 'three', test: /[\\/]node_modules[\\/]three[\\/]/ }] },
            },
          },
        }
      : undefined,
  server: {
    port: 5179,
    proxy: {
      '/api': 'http://127.0.0.1:8787',
    },
    allowedHosts: ['.orb.local', 'localhost'],
  },
}));

import path from 'node:path';
import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

export default defineConfig(({ command }) => ({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, 'src'),
    },
  },
  // Strip `console.*` and `debugger` from production bundles only. The dev
  // server (`vite dev`) keeps both so authors still see logs in iteration.
  esbuild: command === 'build' ? { drop: ['console', 'debugger'] } : undefined,
  server: {
    port: 5179,
    proxy: {
      '/api': 'http://127.0.0.1:8787',
    },
    allowedHosts: ['.orb.local', 'localhost'],
  },
}));

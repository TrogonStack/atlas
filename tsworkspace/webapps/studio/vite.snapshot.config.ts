// Builds the runtime a saved board embeds: one script and one stylesheet
// with fixed names, so Studio can fetch and inline them. The output lands in
// public/ so both `vite dev` and the production build serve it.
import path from 'node:path';
import { defineConfig, mergeConfig, type UserConfigFnObject } from 'vite';
import studio from './vite.config.ts';

export default defineConfig((env) =>
  mergeConfig((studio as UserConfigFnObject)(env), {
    publicDir: false,
    build: {
      outDir: 'public/snapshot',
      emptyOutDir: true,
      cssCodeSplit: false,
      // The whole board runtime is one file on purpose; it is inlined, not fetched in parts.
      chunkSizeWarningLimit: 2048,
      assetsInlineLimit: () => true,
      rolldownOptions: {
        input: path.resolve(import.meta.dirname, 'snapshot.html'),
        output: {
          codeSplitting: false,
          entryFileNames: 'runtime.js',
          assetFileNames: 'runtime[extname]',
        },
      },
    },
  }),
);

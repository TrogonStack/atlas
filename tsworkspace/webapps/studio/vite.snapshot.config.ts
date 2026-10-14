// Builds the runtime a saved board embeds: one script and one stylesheet
// with fixed names, so Studio can fetch and inline them. The output lands in
// public/ so both `vite dev` and the production build serve it.
import path from 'node:path';
import { defineConfig, mergeConfig, type UserConfigFnObject } from 'vite';
import studio from './vite.config.ts';

// Modules a saved board never runs, swapped for stubs by resolved path.
const STUBS: [RegExp, string][] = [
  [/\/src\/lib\/realtime(\.ts)?$/, path.resolve(import.meta.dirname, 'src/lib/realtime.snapshot.ts')],
  [/\/src\/lib\/export-html(\.ts)?$/, path.resolve(import.meta.dirname, 'src/lib/export-html.snapshot.ts')],
];

export default defineConfig((env) =>
  mergeConfig((studio as UserConfigFnObject)(env), {
    publicDir: false,
    plugins: [
      {
        name: 'snapshot-stubs',
        enforce: 'pre',
        // Runs after the `@` alias, so it sees the resolved path.
        resolveId: (id: string) => STUBS.find(([pattern]) => pattern.test(id))?.[1],
      },
    ],
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

import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig, type Plugin } from 'vite';

const RUNTIME_TYPES = new Map([
  ['runtime.js', 'text/javascript'],
  ['runtime.css', 'text/css'],
]);

// Builds into a scratch directory, not public/, so a dev container writes
// nothing into the source tree it mounts.
async function buildSnapshotRuntime(root: string): Promise<Map<string, Buffer>> {
  const outDir = await mkdtemp(path.join(tmpdir(), 'studio-snapshot-'));
  try {
    // A build inside the dev server would inherit its NODE_ENV and bundle React for development.
    await promisify(execFile)(
      process.execPath,
      [
        path.join(root, 'node_modules/vite/bin/vite.js'),
        'build',
        '-c',
        'vite.snapshot.config.ts',
        '--outDir',
        outDir,
        '--emptyOutDir',
        '--logLevel',
        'warn',
      ],
      { cwd: root, env: { ...process.env, NODE_ENV: 'production' } },
    );
    const names = [...RUNTIME_TYPES.keys()];
    return new Map(
      await Promise.all(names.map(async (name) => [name, await readFile(path.join(outDir, name))] as const)),
    );
  } finally {
    await rm(outDir, { recursive: true, force: true });
  }
}

// The dev server builds the saved-board runtime when an export first asks for
// it and again after any file change, so export works without build:snapshot.
function snapshotRuntime(): Plugin {
  let runtime: Promise<Map<string, Buffer>> | undefined;
  return {
    name: 'snapshot-runtime',
    apply: 'serve',
    configureServer(server) {
      server.watcher.on('all', () => {
        runtime = undefined;
      });
      server.middlewares.use('/snapshot', (req, res, next) => {
        const name = req.url?.split('?')[0].slice(1) ?? '';
        const type = RUNTIME_TYPES.get(name);
        if (!type) return next();
        runtime ??= buildSnapshotRuntime(server.config.root);
        runtime.then(
          (files) => {
            res.setHeader('Content-Type', type);
            res.setHeader('Cache-Control', 'no-cache');
            res.end(files.get(name));
          },
          (e: unknown) => {
            runtime = undefined;
            next(e);
          },
        );
      });
    },
  };
}

export default defineConfig(({ command }) => ({
  plugins: [react(), tailwindcss(), snapshotRuntime()],
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

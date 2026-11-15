// Dev-server plugin that builds the saved-board runtime when an export asks
// for it, so export works without build:snapshot. vite.snapshot.config.ts
// merges vite.config.ts, so `apply: 'serve'` keeps this out of that build.
import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import type { Connect, Plugin } from 'vite';

const VITE_BIN = path.join(path.dirname(createRequire(import.meta.url).resolve('vite/package.json')), 'bin/vite.js');

type Runtime = { js: Buffer; css: Buffer };

async function build(root: string): Promise<Runtime> {
  // A scratch directory, not public/, so a dev container writes nothing into
  // the source tree it mounts.
  const outDir = await mkdtemp(path.join(tmpdir(), 'studio-snapshot-'));
  try {
    // A child process, because a build inside the dev server inherits its
    // NODE_ENV and bundles React for development.
    await promisify(execFile)(
      process.execPath,
      [VITE_BIN, 'build', '-c', 'vite.snapshot.config.ts', '--outDir', outDir, '--logLevel', 'warn'],
      { cwd: root, env: { ...process.env, NODE_ENV: 'production' } },
    );
    const [js, css] = await Promise.all(['runtime.js', 'runtime.css'].map((name) => readFile(path.join(outDir, name))));
    return { js, css };
  } finally {
    await rm(outDir, { recursive: true, force: true });
  }
}

export function snapshotRuntime(): Plugin {
  let runtime: Promise<Runtime> | undefined;
  return {
    name: 'snapshot-runtime',
    apply: 'serve',
    configureServer(server) {
      const { root } = server.config;
      const sources = [path.join(root, 'src') + path.sep, path.join(root, 'snapshot.html')];
      // A source change drops the build, so an export never embeds a stale runtime.
      server.watcher.on('all', (_event, file) => {
        if (sources.some((source) => file.startsWith(source))) runtime = undefined;
      });
      const serve: Connect.NextHandleFunction = async (_req, res, next) => {
        try {
          runtime ??= build(root);
          const { js, css } = await runtime;
          res.setHeader('Content-Type', 'application/json');
          res.setHeader('Cache-Control', 'no-cache');
          res.end(JSON.stringify({ js: js.toString('utf8'), css: css.toString('utf8') }));
        } catch (e) {
          runtime = undefined;
          next(e);
        }
      };
      server.middlewares.use('/snapshot/runtime', serve);
    },
  };
}

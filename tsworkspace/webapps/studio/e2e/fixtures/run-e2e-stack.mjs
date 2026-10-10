// Playwright's `webServer` option runs one shell command and waits for one
// URL. The real stack is three processes (fake gRPC backend, the Express
// bridge, and Vite), so this launcher starts the fake gRPC server in-process
// first, learns its ephemeral port, then spawns the bridge and Vite as real
// child processes wired to it via TROGON_ATLAS_GRPC -- the same environment
// variable the bridge already reads in normal dev/prod use (see
// server/index.mjs and server/README.md). Nothing about the bridge or Vite
// config is test-only; this script only chooses what backend they point at.
import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { startFakeGrpcServer } from './fake-grpc-server.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const studioRoot = path.resolve(__dirname, '../..');

const { url: grpcUrl, stop: stopGrpc } = await startFakeGrpcServer();
process.stderr.write(`[e2e] fake gRPC backend listening at ${grpcUrl}\n`);

const sharedEnv = {
  ...process.env,
  TROGON_ATLAS_GRPC: grpcUrl,
  PORT: '8787',
  HOST: '127.0.0.1',
};

/** @type {import('node:child_process').ChildProcess[]} */
const children = [];

/** @param {string} command @param {string[]} args */
function spawnChild(command, args) {
  const child = spawn(command, args, {
    cwd: studioRoot,
    env: sharedEnv,
    stdio: 'inherit',
  });
  children.push(child);
  return child;
}

const bridge = spawnChild(process.execPath, ['server/index.mjs']);
// --host 127.0.0.1 forces an IPv4 bind: spawned outside a shell/TTY, Vite's
// default `localhost` host resolves to the IPv6 loopback only on this
// machine, which the bridge (and Playwright's baseURL) can't reach at
// 127.0.0.1.
const vite = spawnChild(
  path.resolve(studioRoot, 'node_modules/.bin/vite'),
  ['--host', '127.0.0.1'],
);

let shuttingDown = false;
async function shutdown(code) {
  if (shuttingDown) return;
  shuttingDown = true;
  for (const child of children) {
    if (!child.killed) child.kill('SIGTERM');
  }
  await stopGrpc();
  process.exitCode = code;
}

for (const child of [bridge, vite]) {
  child.on('exit', (code) => {
    if (!shuttingDown) {
      process.stderr.write(`[e2e] child process exited early with code ${code}\n`);
      shutdown(code ?? 1);
    }
  });
}

process.on('SIGTERM', () => shutdown(0));
process.on('SIGINT', () => shutdown(0));

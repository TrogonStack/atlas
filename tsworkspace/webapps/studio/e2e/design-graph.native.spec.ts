import { spawn, execFileSync, type ChildProcess } from 'node:child_process';
import { createServer as netServer } from 'node:net';
import { mkdir, readFile, writeFile, realpath, rm } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { createServer } from 'vite';

async function freePort(): Promise<number> {
  const server = netServer();
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('No loopback address');
  await new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  return address.port;
}

test('authenticated native DesignGraph identities survive the production bridge', async ({ page, request }) => {
  const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
  const evidenceDirectory = process.env.ATLAS_NATIVE_EVIDENCE_DIR;
  if (!evidenceDirectory) throw new Error('Missing evidence directory');
  const evidence = path.resolve(evidenceDirectory);
  await mkdir(evidence, { recursive: true });
  if (!(await realpath(evidence)).startsWith(`${await realpath(root)}/.trogonai/`))
    throw new Error('Evidence must remain under .trogonai');
  const receipt = path.join(evidence, 'fixture.internal.trogonai.md');
  await rm(receipt, { force: true });
  const children: ChildProcess[] = [];
  const logs: string[] = [];
  function child(command: string, args: string[], env: NodeJS.ProcessEnv) {
    logs.push(JSON.stringify({ command, args }));
    const process = spawn(command, args, { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] });
    children.push(process);
    process.stdout?.on('data', (data) => logs.push(String(data)));
    process.stderr?.on('data', (data) => logs.push(String(data)));
    return process;
  }
  let vite: Awaited<ReturnType<typeof createServer>> | undefined;
  const diagnostics: unknown[] = [];
  const errors: string[] = [];
  page.on('console', (message) => {
    diagnostics.push({ console: message.type(), text: message.text() });
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('response', (response) => diagnostics.push({ url: response.url(), status: response.status() }));
  page.on('requestfailed', (request) => diagnostics.push({ url: request.url(), failure: request.failure() }));
  try {
    const backend = child(
      'mise',
      [
        'exec',
        '--',
        'cargo',
        'test',
        '--manifest-path',
        'rsworkspace/Cargo.toml',
        '-p',
        'trogon-atlas-server',
        '--test',
        'connected_architecture_fixture',
        'serve_connected_architecture_fixture',
        '--',
        '--ignored',
        '--exact',
        '--nocapture',
      ],
      {
        ...process.env,
        ATLAS_CONNECTED_FIXTURE_GRPC_PORT: '0',
        ATLAS_CONNECTED_FIXTURE_MCP_PORT: '0',
        ATLAS_CONNECTED_FIXTURE_SECONDS: '600',
        ATLAS_BROWSER_FIXTURE_FILE: receipt,
      },
    );
    await expect
      .poll(
        async () => {
          if (backend.exitCode !== null) throw new Error(`Native fixture exited: ${logs.join('')}`);
          return readFile(receipt, 'utf8').catch(() => '');
        },
        { timeout: 480_000 },
      )
      .not.toBe('');
    const fixture = JSON.parse(await readFile(receipt, 'utf8'));
    const bridgePort = await freePort();
    child(process.execPath, ['tsworkspace/webapps/studio/server/index.mjs'], {
      ...process.env,
      TROGON_ATLAS_GRPC: fixture.grpc,
      TROGON_ATLAS_AUTH_PASSTHROUGH: 'true',
      TROGON_ATLAS_AUTH_TOKEN: 'connected-fixture-admin',
      PORT: String(bridgePort),
      HOST: '127.0.0.1',
      NATS_WS_AUTH_TOKEN: 'native-fixture-sse',
    });
    vite = await createServer({
      server: { host: '127.0.0.1', port: 0, proxy: { '/api': `http://127.0.0.1:${bridgePort}` } },
    });
    await vite.listen();
    const address = vite.httpServer?.address();
    if (!address || typeof address === 'string') throw new Error('Missing Vite listener');
    const origin = `http://127.0.0.1:${address.port}`;
    await expect
      .poll(async () =>
        request
          .get(`${origin}/api/event-models`)
          .then((r) => r.status())
          .catch(() => 0),
      )
      .toBe(401);
    await page.addInitScript(() => sessionStorage.setItem('trogon-atlas.credential', 'connected-fixture-admin'));
    for (const entity of fixture.graphs) {
      const graph = entity.designGraph;
      await page.goto(
        `${origin}/em/${fixture.namespace}/model?focus=design_graph:${fixture.namespace}/${graph.id.slug}`,
      );
      await expect(page.getByText('Loaded authored topology', { exact: true })).toBeVisible();
      const section = page
        .locator('section')
        .filter({ has: page.getByRole('heading', { name: 'Design topology inspection', exact: true }) })
        .last();
      await expect(section).toContainText(entity.system.uid);
      for (const step of graph.steps) {
        await expect(section).toContainText(step.stepId.value);
        for (const transition of step.transitions ?? [])
          await expect(section).toContainText(transition.connectionId.value);
      }
      await expect(section).toContainText('Browser customer');
      await expect(section).toContainText(
        graph.profile === 1 ? 'DESIGN_GRAPH_PROFILE_FLOW' : 'DESIGN_GRAPH_PROFILE_DIAGRAM',
      );
      const detail = section
        .locator('section')
        .filter({ has: page.getByRole('heading', { name: 'Design graph detail', exact: true }) });
      await expect(detail).toContainText('CATALOG_RESOLUTION_STATE_UNAVAILABLE');
      await expect(detail).not.toContainText('Unavailable fixture target');
      await expect(section.getByRole('alert')).toHaveCount(0);
      await expect(section).toContainText(graph.layoutDocument.id.slug);
      await expect(section).toContainText(graph.entrySteps[0].value);
      await page.screenshot({ path: path.join(evidence, `${graph.id.slug}.png`), fullPage: true });
      await writeFile(
        path.join(evidence, `${graph.id.slug}.internal.trogonai.md`),
        await page.locator('body').innerText(),
      );
    }
    expect(errors).toEqual([]);
  } finally {
    if (!page.isClosed()) {
      await page.screenshot({ path: path.join(evidence, 'final.png'), fullPage: true }).catch(() => {});
      await writeFile(
        path.join(evidence, 'final-dom.internal.trogonai.md'),
        await page.locator('body').innerText(),
      ).catch(() => {});
      await page.close();
    }
    await vite?.close();
    for (const process of children.reverse()) {
      if (process.exitCode === null) {
        process.kill('SIGINT');
        await new Promise<void>((resolve) => {
          process.once('exit', () => resolve());
          setTimeout(resolve, 5000).unref();
        });
      }
    }
    await writeFile(
      path.join(evidence, 'run.internal.trogonai.md'),
      JSON.stringify(
        { sha: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(), logs, diagnostics, errors },
        null,
        2,
      ),
    );
  }
});

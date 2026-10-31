import { spawn, execFileSync, type ChildProcess } from 'node:child_process';
import { createServer as netServer } from 'node:net';
import { mkdir, readFile, writeFile, realpath, rm } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, type Locator } from '@playwright/test';
import { createServer } from 'vite';

const TOKEN = 'human-interaction-reader';
interface Selection {
  owner: { kind: string; id: { namespace: string; slug: string; version: string } };
  ownerUid: string;
  id: { value: string };
}
interface Display {
  persona: Selection;
  ui: Selection;
  sourceReadModel: Selection;
}
interface Receipt {
  grpc: string;
  namespace: string;
  model: string;
  inert: string;
  hosting: Selection[];
  issuance: { persona: Selection; ui: Selection; command: Selection };
  display: Display;
  unavailable: { hosting: Selection; display: Display };
}
async function freePort() {
  const server = netServer();
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Missing loopback address');
  await new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  return address.port;
}

test('authenticated native hosting and explicit human tuples preserve checked identity and unavailable state', async ({
  page,
  request,
}) => {
  const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
  const evidenceDirectory = process.env.ATLAS_NATIVE_EVIDENCE_DIR;
  if (!evidenceDirectory) throw new Error('Missing native evidence directory');
  const evidence = path.resolve(evidenceDirectory);
  await mkdir(evidence, { recursive: true });
  if (!(await realpath(evidence)).startsWith(`${await realpath(root)}/.trogonai/`))
    throw new Error('Evidence must remain under .trogonai');
  const receiptPath = path.join(evidence, 'human-fixture.internal.trogonai.md');
  await rm(receiptPath, { force: true });
  const children: ChildProcess[] = [];
  const logs: string[] = [];
  const errors: string[] = [];
  const assertions: string[] = [];
  let vite: Awaited<ReturnType<typeof createServer>> | undefined;
  const child = (command: string, args: string[], env: NodeJS.ProcessEnv) => {
    const process = spawn(command, args, { cwd: root, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    children.push(process);
    process.stdout?.on('data', (value) => logs.push(String(value)));
    process.stderr?.on('data', (value) => logs.push(String(value)));
    return process;
  };
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('pageerror', (error) => errors.push(error.message));
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
        'human_interaction_browser_fixture',
        'serve_human_interaction_fixture',
        '--',
        '--ignored',
        '--nocapture',
      ],
      { ...process.env, ATLAS_BROWSER_FIXTURE_FILE: receiptPath },
    );
    await expect
      .poll(
        async () => {
          if (backend.exitCode !== null) throw new Error(`Native fixture exited: ${logs.join('')}`);
          return readFile(receiptPath, 'utf8').catch(() => '');
        },
        { timeout: 480_000 },
      )
      .not.toBe('');
    const fixture: Receipt = JSON.parse(await readFile(receiptPath, 'utf8'));
    const bridgePort = await freePort();
    child(process.execPath, ['tsworkspace/webapps/studio/server/index.mjs'], {
      ...process.env,
      TROGON_ATLAS_GRPC: fixture.grpc,
      TROGON_ATLAS_AUTH_PASSTHROUGH: 'true',
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
          .then((value) => value.status())
          .catch(() => 0),
      )
      .toBe(401);
    assertions.push('Production bridge refuses unauthenticated model reads');
    await page.addInitScript((token) => sessionStorage.setItem('trogon-atlas.credential', token), TOKEN);
    const heading = page.getByRole('heading', { name: 'UI hosting and human interaction', exact: true });
    const open = async (slug: string) => {
      await page.goto(
        `${origin}/em/${fixture.namespace}/${fixture.model}?focus=component:${fixture.namespace}/${slug}`,
      );
      await expect(heading).toBeVisible();
      return page.locator('section').filter({ has: heading }).last();
    };
    const portal = await open('portal');
    for (const selected of fixture.hosting) {
      await expect(portal).toContainText(selected.ownerUid);
      await expect(portal).toContainText(selected.id.value);
    }
    await expect(portal).toContainText(fixture.inert);
    expect(await page.evaluate(() => (window as unknown as Record<string, unknown>).__hostingUnsafe)).toBeUndefined();
    const check = portal.getByRole('button', { name: 'Check selected interaction' });
    await expect(check).toBeDisabled();
    assertions.push('Persisted equal hosting siblings retain distinct IDs; docs remain inert; no tuple inferred');
    const choose = async (section: Locator, hosting: Selection, display: Display) => {
      await section.getByLabel('Exact hosting occurrence').selectOption(hosting.id.value);
      await section.getByLabel('Human path type').selectOption('display');
      const owner = display.ui.owner.id;
      await section
        .getByLabel('Exact slice owner')
        .selectOption(`uiSlice:${owner.namespace}/${owner.slug}@${owner.version}`);
      await section.getByLabel('Persona occurrence').selectOption(display.persona.id.value);
      await section.getByLabel('UI occurrence').selectOption(display.ui.id.value);
      await section.getByLabel('ReadModel observation occurrence').selectOption(display.sourceReadModel.id.value);
    };
    await choose(portal, fixture.hosting[1], fixture.display);
    const observed = page.waitForResponse((value) => value.url().includes('/api/catalog/human-interaction'));
    await check.click();
    const response = await observed;
    expect(response.status()).toBe(200);
    const result = await response.json();
    expect(result.request).toEqual({ hosting: fixture.hosting[1], display: fixture.display });
    expect(result.state).toBe('CATALOG_RESOLUTION_STATE_RESOLVED');
    expect(result.consistency).toBe('HUMAN_INTERACTION_CONSISTENCY_PER_OWNER_CHECKED_READ_SET');
    expect(result.scope.visited).toHaveLength(5);
    for (const row of result.scope.visited) {
      expect(row.uid).toBeTruthy();
      expect(row.etag).toBeTruthy();
    }
    await expect(portal.getByRole('heading', { name: 'Resolved checked path' })).toBeVisible();
    await expect(portal).toContainText('not an atomic snapshot');
    await expect(portal).toContainText(result.scope.revision);
    await expect(portal).toContainText('source_read_models[1]');
    await writeFile(
      path.join(evidence, 'human-display-response.internal.trogonai.md'),
      JSON.stringify(result, null, 2),
    );
    await page.screenshot({ path: path.join(evidence, 'human-display.png'), fullPage: true });
    assertions.push(
      'Authenticated Reader checks only the explicitly selected display observation and retains exact checked row receipts',
    );
    await portal.getByLabel('Human path type').selectOption('issuance');
    await expect(portal.getByRole('heading', { name: 'Resolved checked path' })).toHaveCount(0);
    await expect(check).toBeDisabled();
    const issuanceOwner = fixture.issuance.ui.owner.id;
    await portal
      .getByLabel('Exact slice owner')
      .selectOption(`commandSlice:${issuanceOwner.namespace}/${issuanceOwner.slug}@${issuanceOwner.version}`);
    await portal.getByLabel('Persona occurrence').selectOption(fixture.issuance.persona.id.value);
    await portal.getByLabel('UI occurrence').selectOption(fixture.issuance.ui.id.value);
    await portal.getByLabel('Command occurrence').selectOption(fixture.issuance.command.id.value);
    await check.click();
    await expect(portal.getByRole('heading', { name: 'Resolved checked path' })).toBeVisible();
    assertions.push('Changing path clears the prior receipt; explicit human issuance resolves separately');
    const unavailable = await open('unavailable-portal');
    await choose(unavailable, fixture.unavailable.hosting, fixture.unavailable.display);
    await unavailable.getByRole('button', { name: 'Check selected interaction' }).click();
    await expect(unavailable.getByRole('heading', { name: 'Unavailable endpoint' })).toBeVisible();
    await expect(unavailable).not.toContainText('Gone UI');
    await page.screenshot({ path: path.join(evidence, 'human-unavailable.png'), fullPage: true });
    assertions.push(
      'Force-deleted UI remains generic unavailable and never receives replacement facts from loaded model',
    );
    expect(errors).toEqual([]);
  } finally {
    if (!page.isClosed()) {
      await writeFile(
        path.join(evidence, 'human-dom.internal.trogonai.md'),
        await page.locator('body').innerText(),
      ).catch(() => {});
      await page.close();
    }
    await vite?.close();
    for (const process of children.reverse()) {
      if (process.exitCode !== null || process.pid === undefined) continue;
      globalThis.process.kill(-process.pid, 'SIGINT');
      await new Promise<void>((resolve) => {
        process.once('exit', () => resolve());
        setTimeout(resolve, 5000).unref();
      });
    }
    await writeFile(
      path.join(evidence, 'human-run.internal.trogonai.md'),
      JSON.stringify(
        { sha: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(), assertions, errors, logs },
        null,
        2,
      ),
    );
  }
});

import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { mkdir, open, readFile, realpath, writeFile } from 'node:fs/promises';
import { createServer as netServer } from 'node:net';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { createServer } from 'vite';

interface ExactId {
  namespace: string;
  slug: string;
  version: string;
}
interface GlossaryFixture {
  grpc_endpoint: string;
  namespace: string;
  published: ExactId;
  draft: ExactId;
  stale: ExactId;
}

async function freePort() {
  const server = netServer();
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Missing loopback address');
  await new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  return address.port;
}

async function stopChild(child: ChildProcess) {
  if (child.exitCode !== null || child.signalCode !== null || child.pid === undefined) return;
  const exited = new Promise<void>((resolve) => child.once('exit', () => resolve()));
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGKILL'] as const) {
    if (child.exitCode !== null || child.signalCode !== null) return;
    try {
      process.kill(-child.pid, signal);
    } catch {
      return;
    }
    await Promise.race([exited, new Promise<void>((resolve) => setTimeout(resolve, 3000).unref())]);
  }
  if (child.exitCode === null && child.signalCode === null) throw new Error('Owned test child did not exit');
}

test('authenticated glossary review preserves exact published, draft and stale-source states without model writes', async ({
  page,
  request,
}) => {
  const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
  const directory = process.env.ATLAS_NATIVE_EVIDENCE_DIR;
  const readerToken = process.env.ATLAS_GLOSSARY_FIXTURE_TOKEN;
  if (!directory || !readerToken)
    throw new Error('Native evidence directory and glossary fixture credential are required');
  const evidence = path.resolve(directory);
  await mkdir(evidence, { recursive: true });
  if (!(await realpath(evidence)).startsWith(`${await realpath(root)}/.trogonai/`))
    throw new Error('Native evidence must remain under .trogonai');
  const receiptPath = path.join(evidence, 'glossary-fixture.internal.trogonai.md');
  await (await open(receiptPath, 'wx')).close();
  const children: ChildProcess[] = [];
  const logs: string[] = [];
  const errors: string[] = [];
  const assertions: string[] = [];
  const apiMethods: string[] = [];
  let vite: Awaited<ReturnType<typeof createServer>> | undefined;
  const child = (command: string, args: string[], env: NodeJS.ProcessEnv) => {
    const running = spawn(command, args, { cwd: root, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    children.push(running);
    running.stdout?.on('data', (value) => logs.push(String(value)));
    running.stderr?.on('data', (value) => logs.push(String(value)));
    return running;
  };
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('request', (outgoing) => {
    if (outgoing.url().includes('/api/')) apiMethods.push(outgoing.method());
  });
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
        'glossary_backend',
        'glossary_native_fixture',
        '--',
        '--ignored',
        '--exact',
        '--nocapture',
      ],
      { ...process.env, ATLAS_BROWSER_FIXTURE_FILE: receiptPath },
    );
    await expect
      .poll(
        async () => {
          if (backend.exitCode !== null) throw new Error(`Native fixture exited before readiness: ${logs.join('')}`);
          return readFile(receiptPath, 'utf8').catch(() => '');
        },
        { timeout: 480_000 },
      )
      .not.toBe('');
    const fixture: GlossaryFixture = JSON.parse(await readFile(receiptPath, 'utf8'));
    const bridgePort = await freePort();
    child(process.execPath, ['tsworkspace/webapps/studio/server/index.mjs'], {
      ...process.env,
      TROGON_ATLAS_GRPC: fixture.grpc_endpoint,
      TROGON_ATLAS_AUTH_PASSTHROUGH: 'true',
      TROGON_ATLAS_AUTH_TOKEN: readerToken,
      PORT: String(bridgePort),
      HOST: '127.0.0.1',
    });
    await expect
      .poll(async () =>
        request
          .get(`http://127.0.0.1:${bridgePort}/api/info`)
          .then((response) => response.status())
          .catch(() => 0),
      )
      .toBe(200);
    vite = await createServer({
      server: { host: '127.0.0.1', port: 0, proxy: { '/api': `http://127.0.0.1:${bridgePort}` } },
    });
    await vite.listen();
    const address = vite.httpServer?.address();
    if (!address || typeof address === 'string') throw new Error('Missing Vite address');
    const origin = `http://127.0.0.1:${address.port}`;
    const indexRequest = { selection: { namespaces: [fixture.namespace], includeUnreviewed: true } };
    const indexUrl = `${origin}/api/glossary/entries?request=${encodeURIComponent(JSON.stringify(indexRequest))}`;
    await expect
      .poll(async () =>
        request
          .get(indexUrl)
          .then((response) => response.status())
          .catch(() => 0),
      )
      .toBe(401);
    const refusedWrite = await request.post(`${origin}/api/glossary/term`, {
      data: { term: { id: fixture.published } },
    });
    expect(refusedWrite.status()).toBe(403);
    assertions.push('Real bridge refuses unauthenticated glossary reads and every model write before auth');
    await page.addInitScript((token) => sessionStorage.setItem('trogon-atlas.credential', token), readerToken);
    await page.context().grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
    const open = async (id: ExactId) => {
      const observed = page.waitForResponse((response) => response.url().includes('/api/glossary/term?'));
      await page.goto(
        `${origin}/glossary?${new URLSearchParams({ namespace: id.namespace, slug: id.slug, version: id.version })}`,
      );
      const response = await observed;
      expect(response.status()).toBe(200);
      const result = await response.json();
      expect(result.entry.term.id).toEqual(id);
      expect(result.etag).toBeTruthy();
      expect(result.entry.ownerUid).toBeTruthy();
      await expect(page.getByRole('region', { name: 'Selected glossary entry' })).toBeVisible();
      return result;
    };
    const published = await open(fixture.published);
    expect(published.entry.state).toBe('GLOSSARY_ENTRY_STATE_REVIEWED');
    expect(published.entry.published).toBe(true);
    expect(published.evidence[0].state).toBe('GLOSSARY_EVIDENCE_STATE_CURRENT');
    const panel = page.getByRole('region', { name: 'Selected glossary entry' });
    await expect(panel.getByText('Published selection', { exact: true })).toBeVisible();
    await expect(panel.getByText('Current', { exact: true })).toBeVisible();
    expect(published.entity.term.glossary.usagePeriod).toEqual({
      validFrom: { value: '2026-11-01' },
      validUntil: { value: '2026-12-01' },
    });
    const permittedUse = panel.getByRole('region', { name: 'Permitted term use' });
    await expect(permittedUse).toContainText('2026-11-01');
    await expect(permittedUse).toContainText('2026-12-01');
    assertions.push(
      'The exact retained term-use period is decoded and displayed separately from policy and review dates',
    );
    await panel.getByRole('button', { name: 'Inspect source' }).click();
    await expect(panel.getByText('Current source inspection', { exact: true })).toBeVisible();
    const usageObserved = page.waitForResponse((response) => response.url().includes('/api/glossary/usage?'));
    await panel.getByRole('button', { name: 'shop/purchase-recorded@1', exact: true }).click();
    const usageResponse = await usageObserved;
    expect(usageResponse.status()).toBe(200);
    expect(JSON.parse(new URL(usageResponse.url()).searchParams.get('request') ?? '')).toEqual({
      kind: 'ENTITY_KIND_EVENT',
      id: { namespace: fixture.namespace, slug: 'purchase-recorded', version: '1' },
    });
    const modeledUsage = await usageResponse.json();
    expect(modeledUsage.entity.event.id).toEqual({
      namespace: fixture.namespace,
      slug: 'purchase-recorded',
      version: '1',
    });
    await expect(panel.getByRole('heading', { name: 'Purchase recorded', exact: true })).toBeVisible();
    assertions.push(
      'Modeled usage reads the exact declared Event version through the production GetEntity bridge and renders its real persisted title',
    );
    await panel.getByLabel('Correction request').fill('Clarify that acceptance precedes fulfillment.');
    await panel.getByRole('button', { name: 'Copy correction context' }).click();
    const correction = await page.evaluate(() => navigator.clipboard.readText());
    expect(correction).toContain(
      `${fixture.published.namespace}/${fixture.published.slug}@${fixture.published.version}`,
    );
    expect(correction).toContain(published.etag);
    expect(correction).toContain('Clarify that acceptance precedes fulfillment.');
    expect(correction).not.toContain(readerToken);
    assertions.push(
      'Authenticated reader inspects an exact published entry and current source, then copies revision-bound correction context without credential disclosure',
    );
    const draft = await open(fixture.draft);
    expect(draft.entry.state).toBe('GLOSSARY_ENTRY_STATE_DRAFT');
    expect(draft.entry.published ?? false).toBe(false);
    await expect(panel.getByText('Draft', { exact: true })).toBeVisible();
    await expect(panel.getByText('Not in published selection', { exact: true })).toBeVisible();
    const stale = await open(fixture.stale);
    expect(stale.entry.state).toBe('GLOSSARY_ENTRY_STATE_SOURCE_CHANGED');
    expect(stale.entry.published).toBe(true);
    expect(stale.evidence[0].state).toBe('GLOSSARY_EVIDENCE_STATE_CHANGED');
    await expect(panel.getByText('Source changed', { exact: true })).toBeVisible();
    await expect(panel.getByText('Changed', { exact: true })).toBeVisible();
    assertions.push(
      'Draft v2 does not replace reviewed v1; changed source is rendered independently from recorded expert review',
    );
    const includeCandidates = page.getByRole('checkbox', { name: 'Include unreviewed candidates' });
    await includeCandidates.focus();
    await expect(includeCandidates).toBeFocused();
    await page.keyboard.press('Space');
    await expect(includeCandidates).not.toBeChecked();
    assertions.push('Keyboard operation changes the candidate filter and clears the previous exact selection');
    await expect(page.getByRole('region', { name: 'Selected glossary entry' })).toHaveCount(0);
    const entries = page.getByRole('region', { name: 'Glossary entries' });
    await expect(entries.getByRole('button', { name: /shop\/purchase@1/ })).toBeVisible();
    await expect(entries.getByRole('button', { name: /shop\/purchase@2/ })).toHaveCount(0);
    await expect(entries.getByRole('button', { name: /shop\/stale@1/ })).toBeVisible();
    await expect(entries.getByRole('button', { name: /shop\/stale@1/ })).toContainText('Source changed');
    await page.getByRole('textbox', { name: 'Audience', exact: true }).fill('customer');
    await page.getByRole('combobox', { name: 'Compare with context' }).selectOption(fixture.namespace);
    await page.getByRole('textbox', { name: 'Comparison audience', exact: true }).fill('code');
    const comparisonObserved = page.waitForResponse((response) => response.url().includes('/api/glossary/compare?'));
    await page.getByRole('button', { name: 'Compare contexts', exact: true }).click();
    const comparisonResponse = await comparisonObserved;
    expect(comparisonResponse.status()).toBe(200);
    const comparisonRequest = JSON.parse(new URL(comparisonResponse.url()).searchParams.get('request') ?? '');
    expect(comparisonRequest.left).toMatchObject({ namespaces: [fixture.namespace], audience: 'customer' });
    expect(comparisonRequest.right).toMatchObject({ namespaces: [fixture.namespace], audience: 'code' });
    expect(comparisonRequest.left.includeUnreviewed ?? false).toBe(false);
    expect(comparisonRequest.right.includeUnreviewed ?? false).toBe(false);
    await expect(page.getByText('Differences are candidates for review, not language rulings.')).toBeVisible();
    const comparison = await comparisonResponse.json();
    assertions.push('Authenticated comparison preserves independent customer/code audiences in the same exact context');
    await expect(page.getByRole('button', { name: /^(approve|publish|save|delete)(?:\s|$)/i })).toHaveCount(0);
    expect(apiMethods.every((method) => method === 'GET')).toBe(true);
    expect(errors).toEqual([]);
    await writeFile(
      path.join(evidence, 'glossary-responses.internal.trogonai.md'),
      JSON.stringify({ published, draft, stale, modeledUsage, comparison }, null, 2),
    );
    assertions.push(
      'Published-only filter retains reviewed v1 and stale published content with its warning, excludes draft v2, and exposes only GET reads without model mutation controls',
    );
  } finally {
    await page.close();
    await vite?.close();
    for (const running of children.reverse()) await stopChild(running);
    await writeFile(
      path.join(evidence, 'glossary-run.internal.trogonai.md'),
      JSON.stringify(
        {
          sha: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
          assertions,
          errors,
          apiMethods,
          logs,
          children: children.map((running) => ({ exitCode: running.exitCode, signalCode: running.signalCode })),
        },
        null,
        2,
      ),
    );
  }
});

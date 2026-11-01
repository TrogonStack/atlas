import { spawn, execFileSync, type ChildProcess } from 'node:child_process';
import { createServer as netServer } from 'node:net';
import { mkdir, readFile, writeFile, realpath, rm } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, type Locator } from '@playwright/test';
import { createServer } from 'vite';

const TOKEN = 'channel-storage-admin';

type WireId = { slug?: string; value?: string };
type WireClaim = {
  connectionId: WireId;
  portId: WireId;
  attachment: { ownerUid: string; id: WireId; owner: { id: WireId } };
};
type WireLink = {
  connectionId: WireId;
  kind: string;
  parameterValues: Record<string, string>;
  doc: WireId;
};
type WireEntity = {
  system: { uid: string };
  channel: { links: WireLink[] };
  component: { storageClaims: WireClaim[] };
  dataProduct: { storageClaims: WireClaim[] };
};
type WireBody = { id?: WireId } | undefined;

async function freePort(): Promise<number> {
  const server = netServer();
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('No loopback address');
  await new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  return address.port;
}

interface Identity {
  slug: string;
  uid: string;
}
interface Receipt {
  grpc: string;
  namespace: string;
  model: string;
  inert: string;
  hub: { entity: Identity; links: string[] };
  checkout: { entity: Identity; claims: string[] };
  staleHolder: { entity: Identity; claims: string[] };
  danglingHolder: { entity: Identity; claims: string[] };
  payments: { entity: Identity; claims: string[] };
  offering: { entity: Identity; claims: string[]; port: string };
  outside: Identity;
  warehouse: { entity: Identity; attachments: string[] };
  replaced: { stale: string; current: string };
  decision: Identity;
  addressed: Identity;
  gone: string;
  unloaded: string;
  doomed: string;
}

test('authenticated native Channel and storage declarations keep exact identities through the production bridge', async ({
  page,
  request,
}) => {
  const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
  const evidenceDirectory = process.env.ATLAS_NATIVE_EVIDENCE_DIR;
  if (!evidenceDirectory) throw new Error('Missing evidence directory');
  const evidence = path.resolve(evidenceDirectory);
  await mkdir(evidence, { recursive: true });
  if (!(await realpath(evidence)).startsWith(`${await realpath(root)}/.trogonai/`))
    throw new Error('Evidence must remain under .trogonai');
  const receiptFile = path.join(evidence, 'fixture.internal.trogonai.md');
  await rm(receiptFile, { force: true });
  const children: ChildProcess[] = [];
  const logs: string[] = [];
  function child(command: string, args: string[], env: NodeJS.ProcessEnv) {
    logs.push(JSON.stringify({ command, args }));
    const process = spawn(command, args, { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'], detached: true });
    children.push(process);
    process.stdout?.on('data', (data) => logs.push(String(data)));
    process.stderr?.on('data', (data) => logs.push(String(data)));
    return process;
  }
  let vite: Awaited<ReturnType<typeof createServer>> | undefined;
  const diagnostics: unknown[] = [];
  const errors: string[] = [];
  const assertions: string[] = [];
  const note = (message: string) => assertions.push(message);
  page.on('console', (message) => {
    diagnostics.push({ console: message.type(), text: message.text() });
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('response', (response) => diagnostics.push({ url: response.url(), status: response.status() }));
  page.on('requestfailed', (failed) => diagnostics.push({ url: failed.url(), failure: failed.failure() }));
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
        'channel_storage_browser_fixture',
        'serve_channel_storage_fixture',
        '--',
        '--ignored',
        '--exact',
        '--nocapture',
      ],
      {
        ...process.env,
        ATLAS_CHANNEL_STORAGE_GRPC_PORT: '0',
        ATLAS_CHANNEL_STORAGE_MCP_PORT: '0',
        ATLAS_CHANNEL_STORAGE_SECONDS: '600',
        ATLAS_BROWSER_FIXTURE_FILE: receiptFile,
      },
    );
    await expect
      .poll(
        async () => {
          if (backend.exitCode !== null) throw new Error(`Native fixture exited: ${logs.join('')}`);
          return readFile(receiptFile, 'utf8').catch(() => '');
        },
        { timeout: 480_000 },
      )
      .not.toBe('');
    const fixture: Receipt = JSON.parse(await readFile(receiptFile, 'utf8'));
    const ns = fixture.namespace;
    const bridgePort = await freePort();
    child(process.execPath, ['tsworkspace/webapps/studio/server/index.mjs'], {
      ...process.env,
      TROGON_ATLAS_GRPC: fixture.grpc,
      TROGON_ATLAS_AUTH_PASSTHROUGH: 'true',
      TROGON_ATLAS_AUTH_TOKEN: TOKEN,
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
    note('bridge rejects an unauthenticated read with 401');

    // Bridge wire receipt: what the browser is given must carry the persisted identities untouched.
    const wire = await request.get(`${origin}/api/event-model/${ns}/${fixture.model}`, {
      headers: { authorization: `Bearer ${TOKEN}` },
    });
    expect(wire.status()).toBe(200);
    const bridged = (await wire.json()) as { entities: WireEntity[] };
    await writeFile(path.join(evidence, 'bridge-model.internal.trogonai.md'), JSON.stringify(bridged, null, 2));
    const bodies = (e: WireEntity) => Object.values(e as unknown as Record<string, WireBody>);
    const bySlug = (key: string, slug: string): WireEntity => {
      const found = bridged.entities.find((e) => (e as unknown as Record<string, WireBody>)[key]?.id?.slug === slug);
      if (found === undefined) throw new Error(`no ${key} ${slug} in the bridge payload`);
      return found;
    };
    const hubWire = bySlug('channel', fixture.hub.entity.slug);
    expect(hubWire.system.uid).toBe(fixture.hub.entity.uid);
    expect(hubWire.channel.links.map((l) => l.connectionId.value)).toEqual(fixture.hub.links);
    expect(hubWire.channel.links.map((l) => l.kind)).toEqual([
      'CHANNEL_LINK_KIND_COMPOSES',
      'CHANNEL_LINK_KIND_COMPOSES',
      'CHANNEL_LINK_KIND_ROUTES_TO',
      'CHANNEL_LINK_KIND_ROUTES_TO',
      'CHANNEL_LINK_KIND_ROUTES_TO',
    ]);
    expect(hubWire.channel.links[0].parameterValues).toEqual({ tenant: 'acme' });
    expect(hubWire.channel.links[1].parameterValues).toEqual({ tenant: 'acme' });
    expect(hubWire.channel.links[2].parameterValues).toEqual({ tenant: 'globex' });
    expect(hubWire.channel.links[0].doc.value).toBe(fixture.inert);
    const checkoutWire = bySlug('component', fixture.checkout.entity.slug);
    expect(checkoutWire.system.uid).toBe(fixture.checkout.entity.uid);
    expect(checkoutWire.component.storageClaims.map((c) => c.connectionId.value)).toEqual(fixture.checkout.claims);
    expect(checkoutWire.component.storageClaims[0].attachment.ownerUid).toBe(fixture.warehouse.entity.uid);
    expect(checkoutWire.component.storageClaims[0].attachment.id.value).toBe(fixture.warehouse.attachments[1]);
    const productWire = bySlug('dataProduct', fixture.offering.entity.slug);
    expect(productWire.dataProduct.storageClaims[0].portId.value).toBe(fixture.offering.port);
    const staleWire = bySlug('component', fixture.staleHolder.entity.slug);
    expect(staleWire.component.storageClaims[0].attachment.owner.id.slug).toBe('replaced');
    expect(staleWire.component.storageClaims[0].attachment.ownerUid).toBe(fixture.replaced.stale);
    expect(bySlug('dataStore', 'replaced').system.uid).toBe(fixture.replaced.current);
    expect(fixture.replaced.stale).not.toBe(fixture.replaced.current);
    for (const slug of [fixture.gone, fixture.doomed, fixture.unloaded, fixture.outside.slug])
      expect(bridged.entities.some((e) => bodies(e).some((v) => v?.id?.slug === slug))).toBe(false);
    note('bridge payload carries persisted uids, occurrence ids, kinds, parameters, pins, port and doc bytes');

    await page.addInitScript((token) => sessionStorage.setItem('trogon-atlas.credential', token), TOKEN);
    const heading = page.getByRole('heading', { name: 'Channel links and storage claims', exact: true });
    const open = async (short: string, slug: string): Promise<Locator> => {
      await page.goto(`${origin}/em/${ns}/model?focus=${short}:${ns}/${slug}`);
      await expect(heading).toBeVisible();
      const section = page.locator('section').filter({ has: heading }).last();
      await expect(section).toContainText('Owner-local authored associations');
      await expect(section).toContainText(
        'Links and claims are descriptive, and do not establish routing, delivery or runtime behavior',
      );
      await page.screenshot({ path: path.join(evidence, `${slug}.png`), fullPage: true });
      await writeFile(path.join(evidence, `${slug}.internal.trogonai.md`), await section.innerText());
      return section;
    };
    const rowsOf = (section: Locator) => section.locator('section').filter({ hasText: 'declared by the owner' });
    const label = (slug: string) => `${ns}/${slug}@1`;
    const inertScript = '<script>window.__atlasInert = true</script>';
    const expectInert = async (section: Locator) => {
      await expect(section.locator('script')).toHaveCount(0);
      expect(await page.evaluate(() => (window as unknown as Record<string, unknown>).__atlasInert)).toBeUndefined();
    };

    // Channel: equal siblings, parameters, kinds, inert docs, unloaded and deleted targets.
    const hub = await open('channel', fixture.hub.entity.slug);
    const hubRows = rowsOf(hub);
    await expect(hubRows).toHaveCount(5);
    for (const [index, kind] of ['COMPOSES', 'COMPOSES', 'ROUTES_TO', 'ROUTES_TO', 'ROUTES_TO'].entries()) {
      const row = hubRows.nth(index);
      await expect(row).toContainText(`${kind} declared by the owner`);
      await expect(row).toContainText(`links[${index}]`);
      await expect(row).toContainText(fixture.hub.links[index]);
      await expect(row).toContainText(fixture.hub.entity.uid);
      for (const other of fixture.hub.links.filter((_, i) => i !== index)) await expect(row).not.toContainText(other);
    }
    for (const index of [0, 1]) {
      await expect(hubRows.nth(index)).toContainText('tenant=acme');
      await expect(hubRows.nth(index).getByRole('button', { name: label('addressed') })).toBeVisible();
      await expect(hubRows.nth(index)).toContainText(inertScript);
    }
    await expect(hubRows.nth(2)).toContainText('tenant=globex');
    await expect(hubRows.nth(2)).not.toContainText(inertScript);
    await expect(hubRows.nth(3)).toContainText(`${label(fixture.gone)} (not loaded)`);
    await expect(hubRows.nth(3)).toContainText('None');
    await expect(hubRows.nth(4)).toContainText(`${label(fixture.unloaded)} (not loaded)`);
    for (const index of [3, 4]) {
      await expect(hubRows.nth(index).getByRole('button')).toHaveCount(1);
      await expect(hubRows.nth(index).getByRole('button', { name: label(fixture.hub.entity.slug) })).toBeVisible();
    }
    await expect(hub).toContainText('Incoming discovery is limited to loaded owners');
    await expectInert(hub);
    note(
      'Channel: five exact occurrences, equal siblings distinguished by id/path, parameters, inert docs, not-loaded targets',
    );

    // Selecting an exact target navigates inside the loaded scope and lists the loaded incoming links.
    await hubRows
      .nth(0)
      .getByRole('button', { name: label('addressed') })
      .click();
    const addressed = page.locator('section').filter({ has: heading }).last();
    await expect(addressed).toContainText('No links or storage claims declared in this loaded owner.');
    const incoming = rowsOf(addressed);
    await expect(incoming).toHaveCount(3);
    for (const id of fixture.hub.links.slice(0, 3)) await expect(incoming.filter({ hasText: id })).toHaveCount(1);
    for (const id of fixture.hub.links.slice(3)) await expect(addressed).not.toContainText(id);
    await page.screenshot({ path: path.join(evidence, 'addressed-after-select.png'), fullPage: true });
    note('Selecting the addressed Channel lists exactly its three loaded incoming links');

    // Component: READ pinned attachment, equal WRITE siblings.
    const checkout = await open('component', fixture.checkout.entity.slug);
    const checkoutRows = rowsOf(checkout);
    await expect(checkoutRows).toHaveCount(3);
    await expect(checkoutRows.nth(0)).toContainText('READ declared by the owner');
    await expect(checkoutRows.nth(1)).toContainText('WRITE declared by the owner');
    await expect(checkoutRows.nth(2)).toContainText('WRITE declared by the owner');
    for (const [index, id] of fixture.checkout.claims.entries()) {
      await expect(checkoutRows.nth(index)).toContainText(`storageClaims[${index}]`);
      await expect(checkoutRows.nth(index)).toContainText(id);
      await expect(checkoutRows.nth(index)).toContainText(fixture.checkout.entity.uid);
      await expect(checkoutRows.nth(index)).toContainText(inertScript);
    }
    await expect(checkoutRows.nth(0)).toContainText(fixture.warehouse.entity.uid);
    await expect(checkoutRows.nth(0)).toContainText(fixture.warehouse.attachments[1]);
    await expect(checkoutRows.nth(0)).not.toContainText(fixture.warehouse.attachments[0]);
    await expect(checkoutRows.nth(0)).not.toContainText('Unavailable exact attachment');
    await expect(checkoutRows.nth(0).getByRole('button', { name: label('warehouse') })).toHaveCount(2);
    for (const index of [1, 2]) {
      await expect(checkoutRows.nth(index)).not.toContainText(fixture.warehouse.attachments[1]);
      await expect(checkoutRows.nth(index)).toContainText('None');
    }
    await expect(checkout).toContainText("The store's access mode is not admission authority");
    await expectInert(checkout);
    note('Component: pinned READ attachment exact and available, WRITE siblings distinct, inert docs');

    // Pinned attachment availability when the store was deleted and recreated, and when it is gone.
    const stale = await open('component', fixture.staleHolder.entity.slug);
    await expect(rowsOf(stale)).toHaveCount(1);
    await expect(stale).toContainText(fixture.replaced.stale);
    await expect(stale).not.toContainText(fixture.replaced.current);
    await expect(stale).toContainText('Unavailable exact attachment');
    await expect(stale.getByRole('button', { name: label('replaced') })).toHaveCount(2);
    note('Recreated store: stale pinned attachment is Unavailable, never hydrated against the new incarnation');
    const dangling = await open('component', fixture.danglingHolder.entity.slug);
    await expect(rowsOf(dangling)).toHaveCount(2);
    await expect(dangling).toContainText(`${label(fixture.doomed)} (not loaded)`);
    await expect(dangling).toContainText('Unavailable exact attachment');
    await expect(dangling.getByRole('button')).toHaveCount(2);
    await expect(dangling.getByRole('button', { name: label(fixture.danglingHolder.entity.slug) })).toHaveCount(2);
    for (const id of fixture.danglingHolder.claims) await expect(dangling).toContainText(id);
    note('Deleted store: claims keep their identities, targets render not loaded, no crash');

    // ExternalSystem and DataProduct storage claims.
    const payments = await open('external_system', fixture.payments.entity.slug);
    await expect(rowsOf(payments)).toHaveCount(1);
    await expect(rowsOf(payments).first()).toContainText('READ declared by the owner');
    await expect(rowsOf(payments).first()).toContainText(fixture.payments.claims[0]);
    await expect(
      rowsOf(payments)
        .first()
        .getByRole('button', { name: label('warehouse') }),
    ).toBeVisible();
    const offering = await open('data_product', fixture.offering.entity.slug);
    await expect(rowsOf(offering)).toHaveCount(1);
    await expect(rowsOf(offering).first()).toContainText(fixture.offering.claims[0]);
    await expect(rowsOf(offering).first()).toContainText('Data port');
    await expect(rowsOf(offering).first()).toContainText(fixture.offering.port);
    note('ExternalSystem and DataProduct storage claims keep exact claim ids and the DataProduct port id');

    // DataStore: loaded-only incoming claims, READ and WRITE distinct, unloaded claimant absent.
    const warehouse = await open('data_store', fixture.warehouse.entity.slug);
    await expect(warehouse).toContainText('No links or storage claims declared in this loaded owner.');
    const stored = rowsOf(warehouse);
    await expect(stored).toHaveCount(5);
    for (const id of [...fixture.checkout.claims, ...fixture.payments.claims, ...fixture.offering.claims])
      await expect(stored.filter({ hasText: id })).toHaveCount(1);
    await expect(stored.filter({ hasText: 'READ declared by the owner' })).toHaveCount(3);
    await expect(stored.filter({ hasText: 'WRITE declared by the owner' })).toHaveCount(2);
    await expect(warehouse).not.toContainText(fixture.outside.slug);
    await expect(warehouse).not.toContainText(fixture.outside.uid);
    await expect(warehouse).toContainText('Incoming discovery is limited to loaded owners');
    note('DataStore: five loaded incoming claims, unloaded outside claimant never shown');
    const replaced = await open('data_store', 'replaced');
    const replacedRows = rowsOf(replaced);
    await expect(replacedRows).toHaveCount(1);
    await expect(replacedRows).toContainText(fixture.staleHolder.claims[0]);
    await expect(replacedRows).toContainText('Unavailable exact attachment');

    // Equal sibling exact selection through a Decision subject.
    const decision = await open('decision', fixture.decision.slug);
    await expect(decision).toContainText('Selected associations in the loaded model');
    const selected = rowsOf(decision);
    await expect(selected).toHaveCount(2);
    await expect(selected.nth(0)).toContainText('links[1]');
    await expect(selected.nth(0)).toContainText(fixture.hub.links[1]);
    await expect(selected.nth(1)).toContainText('storageClaims[2]');
    await expect(selected.nth(1)).toContainText(fixture.checkout.claims[2]);
    for (const id of [fixture.hub.links[0], ...fixture.hub.links.slice(2), ...fixture.checkout.claims.slice(0, 2)])
      await expect(decision).not.toContainText(id);
    note('Decision subjects select exactly links[1] and storageClaims[2], not their equal siblings');

    // Missing and deleted focus targets must not crash or invent an inspector.
    for (const [short, slug] of [
      ['channel', fixture.gone],
      ['data_store', fixture.doomed],
      ['channel', fixture.unloaded],
      ['component', fixture.outside.slug],
    ] as const) {
      await page.goto(`${origin}/em/${ns}/model?focus=${short}:${ns}/${slug}`);
      await expect(page.getByText('Channel storage browser acceptance').first()).toBeVisible();
      await expect(page.getByText('Observe receipts').first()).toBeVisible();
      await page.waitForTimeout(1500);
      await expect(heading).toHaveCount(0);
      await expect(page.getByRole('alert')).toHaveCount(0);
      await page.screenshot({ path: path.join(evidence, `missing-${slug}.png`), fullPage: true });
    }
    note('Focusing deleted, unloaded or out-of-scope targets leaves no inspector, no alert and no console error');
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
        if (process.pid !== undefined) globalThis.process.kill(-process.pid, 'SIGINT');
        await new Promise<void>((resolve) => {
          process.once('exit', () => resolve());
          setTimeout(resolve, 5000).unref();
        });
      }
    }
    await writeFile(
      path.join(evidence, 'run.internal.trogonai.md'),
      JSON.stringify(
        {
          sha: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
          assertions,
          logs,
          diagnostics,
          errors,
        },
        null,
        2,
      ),
    );
  }
});

import { readFile } from 'node:fs/promises';
import { expect, test } from '@playwright/test';
import { ORDER_MANIFESTS, ORDER_MODEL_ID, ORDER_PLACED_EVENT } from './fixtures/fake-grpc-server.mjs';

const { namespace, slug } = ORDER_MODEL_ID;
const eventTitle = ORDER_PLACED_EVENT.event.title;

// The point of an exported board is that it opens with nothing else running,
// so the saved file is opened in a fresh page with every network request refused.
test('Export HTML saves a board that renders from disk with no network', async ({ page, browser }, testInfo) => {
  await page.goto(`/em/${namespace}/${slug}`);
  await expect(page.getByText(eventTitle)).toBeVisible();

  const downloading = page.waitForEvent('download');
  await page.getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: 'Export HTML' }).click();
  const download = await downloading;
  expect(download.suggestedFilename()).toMatch(new RegExp(`^${slug}-\\d{8}-\\d{4}\\.html$`));
  // Keep the .html name: Chrome shows a file without it as plain text.
  const file = testInfo.outputPath(download.suggestedFilename());
  await download.saveAs(file);

  const offline = await browser.newPage();
  const problems: string[] = [];
  offline.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
  offline.on('console', (m) => {
    if (m.type() === 'error') problems.push(`console: ${m.text()}`);
  });
  await offline.route(/^https?:/, (route) => {
    problems.push(`network: ${route.request().url()}`);
    return route.abort();
  });
  await offline.goto(`file://${file}`);

  await expect(offline.getByRole('heading', { name: 'Event Model Studio', level: 1 })).toBeVisible();
  await expect(offline.getByText(eventTitle)).toBeVisible();
  await expect(offline.getByRole('status', { name: 'Snapshot' })).toContainText('baseline');
  await expect(offline.getByRole('button', { name: 'More actions' })).toHaveCount(0);

  // The YAML in the file is what `trogon-atlas apply` reads, so the board maps back to manifests.
  const savingYaml = offline.waitForEvent('download');
  await offline.getByRole('button', { name: 'Download YAML' }).click();
  const yaml = await savingYaml;
  expect(yaml.suggestedFilename()).toMatch(new RegExp(`^${slug}-\\d{8}-\\d{4}\\.yaml$`));
  expect(await readFile((await yaml.path()) ?? '', 'utf8')).toBe(ORDER_MANIFESTS);
  expect(problems).toEqual([]);
  // Read from the file: the page drops the data element once it has parsed it.
  const html = await readFile(file, 'utf8');
  // One data tag only, so a reader that takes the first one gets the board, not the runtime's source.
  expect(html.split('<script id="atlas-snapshot"').length - 1).toBe(1);
  const data = JSON.parse(
    html.match(/<script id="atlas-snapshot" type="application\/json">(.*?)<\/script>/s)?.[1] ?? '{}',
  );
  expect(data.manifests).toEqual({ model: ORDER_MANIFESTS, external: '' });
  // The file carries the board's reads and nothing else: no realtime or credential responses.
  expect(Object.keys(data.responses).sort()).toEqual([
    `/api/event-model/${namespace}/${slug}`,
    '/api/info',
    `/api/validate?namespace=${namespace}&slug=${slug}`,
  ]);
  await offline.close();
});

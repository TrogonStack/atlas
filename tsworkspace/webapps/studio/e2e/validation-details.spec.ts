import { expect, test } from '@playwright/test';
import { ORDER_MODEL_ID } from './fixtures/fake-grpc-server.mjs';

const modelPath = `/em/${ORDER_MODEL_ID.namespace}/${ORDER_MODEL_ID.slug}`;

test('validation counters reveal findings and jump to a centered entity at a moderate zoom', async ({ page }) => {
  await page.addInitScript((path) => {
    sessionStorage.setItem(
      `trogon-atlas-studio:viewport:${path}:board`,
      JSON.stringify({ x: -300, y: -200, zoom: 0.3 }),
    );
  }, modelPath);
  await page.goto(modelPath);
  const trigger = page.getByRole('button', { name: 'Validation findings', exact: true });
  const panel = page.getByRole('complementary', { name: 'Validation findings', exact: true });
  const canvasBefore = await page.getByRole('main').boundingBox();

  await trigger.click();
  const bounds = await panel.boundingBox();
  const canvasAfter = await page.getByRole('main').boundingBox();
  if (!bounds || !canvasBefore || !canvasAfter) throw new Error('Drawer or canvas has no visible bounds');
  expect(bounds.x + bounds.width).toBe(page.viewportSize()?.width);
  expect(bounds.y).toBe(canvasAfter.y);
  expect(bounds.height).toBe(canvasAfter.height);
  expect(canvasAfter.width).toBeLessThan(canvasBefore.width);
  expect(canvasAfter.x + canvasAfter.width).toBe(bounds.x);
  await expect(panel.locator('article')).toHaveCount(3);
  await expect(panel.getByText('Customer identifier needs a source.')).toBeVisible();
  await panel.getByRole('button', { name: 'Warnings (1)' }).click();
  await expect(panel.locator('article')).toHaveCount(1);
  await expect(panel.getByText('Explain the customer identifier.')).toBeVisible();

  await page.keyboard.press('Escape');
  await expect(panel).not.toBeVisible();
  await expect(trigger).toBeFocused();
  await page.keyboard.press('Enter');
  await page.waitForTimeout(700);
  const viewportBefore = await page.locator('.react-flow__viewport').getAttribute('style');
  await panel.getByRole('button', { name: 'Open event shop/order.placed@1' }).first().click();
  await expect(panel).not.toBeVisible();
  await expect(page.locator('aside h2')).toHaveText('Order Placed');
  const entityDrawer = await page.locator('aside').boundingBox();
  if (!entityDrawer) throw new Error('Entity drawer has no visible bounds');
  expect(bounds.width).toBe(entityDrawer.width);
  await page.waitForTimeout(700);
  expect(await page.locator('.react-flow__viewport').getAttribute('style')).not.toBe(viewportBefore);
  await expect(page.locator('.react-flow__node.selected')).toHaveCount(1);
  const target = await page.locator('.react-flow__node.selected').boundingBox();
  const canvas = await page.getByRole('main').boundingBox();
  if (!target || !canvas) throw new Error('Target or canvas has no visible bounds');
  expect(Math.abs(target.x + target.width / 2 - (canvas.x + canvas.width / 2))).toBeLessThan(3);
  expect(Math.abs(target.y + target.height / 2 - (canvas.y + canvas.height / 2))).toBeLessThan(3);
  const zoom = await page
    .locator('.react-flow__viewport')
    .evaluate((element) => new DOMMatrixReadOnly(getComputedStyle(element).transform).a);
  expect(zoom).toBeGreaterThanOrEqual(0.8);
  expect(zoom).toBeLessThanOrEqual(1);
  expect(new URL(page.url()).searchParams.get('selected')).toBe('event:shop/order.placed@1');
  await expect(trigger).toBeFocused();
});

test('validation details stay reachable with an inspector on a narrow screen', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${modelPath}?selected=event:shop/order.placed@1`);
  await expect(page.locator('aside h2')).toBeVisible();
  const entityDrawer = await page.locator('aside').boundingBox();
  if (!entityDrawer) throw new Error('Entity drawer has no visible bounds');
  await page.getByRole('button', { name: 'Validation findings', exact: true }).click();
  const panel = page.getByRole('complementary', { name: 'Validation findings', exact: true });
  await expect(panel).toBeVisible();
  await expect(page.locator('aside')).toHaveCount(1);

  for (const height of [844, 400]) {
    await page.setViewportSize({ width: 390, height });
    const bounds = await panel.boundingBox();
    if (!bounds) throw new Error('Validation details have no visible bounds');
    expect(bounds.width).toBe(entityDrawer.width);
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(390);
    expect(bounds.x + bounds.width).toBe(390);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(height);
  }
  await panel.getByRole('button', { name: 'Close validation findings' }).click();
  await expect(panel).not.toBeVisible();
});

test('finding jumps reduce excessive zoom and recenter again after panning away', async ({ page }) => {
  await page.addInitScript((path) => {
    sessionStorage.setItem(`trogon-atlas-studio:viewport:${path}:board`, JSON.stringify({ x: 30, y: 40, zoom: 1.8 }));
  }, modelPath);
  await page.goto(modelPath);
  const trigger = page.getByRole('button', { name: 'Validation findings', exact: true });
  const panel = page.getByRole('complementary', { name: 'Validation findings', exact: true });
  const viewport = page.locator('.react-flow__viewport');

  for (const attempt of [0, 1]) {
    await trigger.click();
    await panel.getByRole('button', { name: 'Open event shop/order.placed@1' }).first().click();
    await expect(page.locator('aside h2')).toHaveText('Order Placed');
    await page.waitForTimeout(700);
    const target = await page.locator('.react-flow__node.selected').boundingBox();
    const canvas = await page.getByRole('main').boundingBox();
    if (!target || !canvas) throw new Error('Target or canvas has no visible bounds');
    expect(Math.abs(target.x + target.width / 2 - (canvas.x + canvas.width / 2))).toBeLessThan(3);
    expect(Math.abs(target.y + target.height / 2 - (canvas.y + canvas.height / 2))).toBeLessThan(3);
    const zoom = await viewport.evaluate((element) => new DOMMatrixReadOnly(getComputedStyle(element).transform).a);
    expect(zoom).toBeGreaterThanOrEqual(0.8);
    expect(zoom).toBeLessThanOrEqual(1);

    if (attempt === 0) {
      const centered = await viewport.getAttribute('style');
      await page.mouse.move(canvas.x + 30, canvas.y + canvas.height - 100);
      await page.mouse.down();
      await page.mouse.move(canvas.x + 200, canvas.y + canvas.height - 60, { steps: 5 });
      await page.mouse.up();
      expect(await viewport.getAttribute('style')).not.toBe(centered);
    }
  }
});

import { expect, test } from '@playwright/test';

test('copies a complete fix prompt from findings and the entity inspector', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', {
      value: {
        writeText: async (text: string) => {
          document.documentElement.dataset.copiedPrompt = text;
        },
      },
    });
  });
  await page.goto('/em/shop/order-service?branch=e2e%2Fsmoke-fixture');
  await page.getByRole('button', { name: 'Validation findings', exact: true }).click();
  const findings = page.getByRole('complementary', { name: 'Validation findings' });
  const finding = findings.locator('article').filter({ hasText: 'E2E_MISSING_SOURCE' });
  await finding.getByRole('button', { name: 'Copy fix prompt', exact: true }).click();
  await expect(finding.getByRole('button', { name: 'Fix prompt copied' })).toBeVisible();
  const prompt = await page.evaluate(() => document.documentElement.dataset.copiedPrompt ?? '');
  expect(prompt).toMatch(/fix.*validation/i);
  expect(prompt).toContain('E2E_MISSING_SOURCE');
  expect(prompt).toContain('Customer identifier needs a source.');
  expect(prompt).toContain('schema.fields.customer_id');
  expect(prompt).toContain('e2e/smoke-fixture');
  expect(prompt).toContain('/em/shop/order-service');
  expect(prompt).toContain('"slug": "order.placed"');
  expect(prompt).toMatch(/rerun validation/i);
  expect(prompt).not.toContain('E2E_SCHEMA_HINT');

  await finding.getByRole('button', { name: 'Open event shop/order.placed@1' }).click();
  await expect(page.locator('aside h2')).toHaveText('Order Placed');
  const errorCard = page.locator('aside div.rounded.border').filter({ hasText: 'Customer identifier needs a source.' });
  await errorCard.getByRole('button', { name: 'Copy fix prompt', exact: true }).click();
  await expect(errorCard.getByRole('button', { name: 'Fix prompt copied' })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.dataset.copiedPrompt)).toBe(prompt);
});

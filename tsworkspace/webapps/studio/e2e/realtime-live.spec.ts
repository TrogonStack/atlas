import { expect, test } from '@playwright/test';

const liveUrl = process.env.TROGON_ATLAS_STUDIO_URL;

test('live update subscriptions stay healthy across tabs and leader closure', async ({ context }) => {
  test.skip(!liveUrl, 'Set TROGON_ATLAS_STUDIO_URL to a model URL on a running compose stack.');
  if (!liveUrl) return;
  const protocolErrors: string[] = [];
  context.on('page', (page) => {
    page.on('websocket', (socket) => {
      socket.on('framereceived', ({ payload }) => {
        const frame = String(payload);
        if (frame.startsWith('-ERR')) protocolErrors.push(frame);
      });
    });
  });
  const first = await context.newPage();
  await first.addInitScript(() => {
    Object.defineProperty(crypto, 'randomUUID', { value: () => '00000000-0000-4000-8000-000000000001' });
  });
  await first.goto(liveUrl);
  await expect(first.getByRole('status', { name: 'Live updates' })).toHaveText('live');
  await first.waitForTimeout(3000);
  expect(protocolErrors).toEqual([]);
  await expect(first.getByRole('status', { name: 'Live updates' })).toHaveText('live');

  const second = await context.newPage();
  await second.addInitScript(() => {
    Object.defineProperty(crypto, 'randomUUID', { value: () => '00000000-0000-4000-8000-000000000002' });
  });
  await second.goto(first.url());
  await expect(second.getByRole('status', { name: 'Live updates' })).toHaveText('live');
  await expect(first.getByRole('status', { name: 'Live updates' })).toHaveText('live');
  await first.close();
  await second.waitForTimeout(4000);
  await expect(second.getByRole('status', { name: 'Live updates' })).toHaveText('live');
  expect(protocolErrors).toEqual([]);
});

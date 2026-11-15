import { expect, test, type Page } from '@playwright/test';
import { ORDER_MODEL_ID } from './fixtures/fake-grpc-server.mjs';

const modelPath = `/em/${ORDER_MODEL_ID.namespace}/${ORDER_MODEL_ID.slug}`;
const id = (slug: string) => ({ namespace: ORDER_MODEL_ID.namespace, slug, version: '1' });
const ref = (slug: string) => ({ id: id(slug) });

async function openLifecycle(page: Page, destinations: string[]) {
  const events = [...new Set(['opened', ...destinations])];
  const entities = [
    { eventModel: { id: ORDER_MODEL_ID, title: 'Lifecycle rendering' } },
    {
      swimlane: {
        id: id('order'),
        title: 'Order',
        transitions: [
          {
            after: ref('opened'),
            next: destinations.map((slug, index) => ({ event: ref(slug), doc: `Guard ${index + 1}` })),
          },
          ...events.filter((slug) => slug !== 'opened').map((slug) => ({ after: ref(slug), next: [] })),
        ],
      },
    },
    ...events.map((slug) => ({ event: { id: id(slug), title: slug, swimlane: ref('order') } })),
  ];
  await page.addInitScript(() => {
    localStorage.setItem('trogon-atlas.studio.feature-flags', JSON.stringify({ stateMachine: true }));
  });
  await page.route(`/api/event-model/${ORDER_MODEL_ID.namespace}/${ORDER_MODEL_ID.slug}`, (route) =>
    route.fulfill({ json: { entities } }),
  );
  await page.goto(
    `${modelPath}?view=states&selected=${encodeURIComponent(`event:${ORDER_MODEL_ID.namespace}/opened@1`)}`,
  );
  await expect(page.getByRole('region', { name: 'Order lifecycle' })).toBeVisible();
  await expect(page.getByTestId('state-machine-next')).toBeVisible();
}

for (const [description, destination] of [
  ['destinations', 'closed'],
  ['returns', 'opened'],
] as const) {
  test(`transition numbers remain readable for repeated ${description}`, async ({ page }) => {
    await openLifecycle(page, [destination, destination, destination, 'other']);
    const diagram = page.getByRole('region', { name: 'Order lifecycle' });
    await expect(diagram.locator('svg g > text')).toHaveText(['1–3', '4']);
    const badges = await diagram.locator('svg g').evaluateAll((elements) =>
      elements.map((element) => {
        const { x, y, width, height } = element.getBoundingClientRect();
        return { x, y, width, height };
      }),
    );
    expect(badges.length).toBeGreaterThan(1);
    for (const [index, badge] of badges.entries()) {
      for (const other of badges.slice(index + 1)) {
        const overlapX = Math.min(badge.x + badge.width, other.x + other.width) - Math.max(badge.x, other.x);
        const overlapY = Math.min(badge.y + badge.height, other.y + other.height) - Math.max(badge.y, other.y);
        expect(overlapX > 0 && overlapY > 0).toBe(false);
      }
    }
    await expect(page.getByTestId('state-machine-next').getByRole('listitem')).toHaveCount(4);
    for (let number = 1; number <= 4; number++) {
      await expect(page.getByTestId('state-machine-next').getByText(`Guard ${number}`, { exact: true })).toBeVisible();
    }
  });
}

test('long transition details scroll while the diagram keeps its space', async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 600 });
  await openLifecycle(
    page,
    Array.from({ length: 80 }, (_, index) => `closed-${index}`),
  );
  const board = page.getByTestId('state-machine');
  const details = page.getByTestId('state-machine-next');
  const boardBounds = await board.boundingBox();
  const detailBounds = await details.boundingBox();
  if (!boardBounds || !detailBounds) throw new Error('Lifecycle board or transition details have no bounds');
  expect(detailBounds.height).toBeLessThanOrEqual(boardBounds.height / 2);
  expect(detailBounds.y + detailBounds.height).toBeLessThanOrEqual(boardBounds.y + boardBounds.height);
  const scroll = await details.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
    return { top: element.scrollTop, height: element.clientHeight, total: element.scrollHeight };
  });
  expect(scroll.total).toBeGreaterThan(scroll.height);
  expect(scroll.top).toBeGreaterThan(0);
  await expect(details.getByText('Guard 80', { exact: true })).toBeInViewport();
  const paneHeight = await page
    .getByRole('region', { name: 'Order lifecycle' })
    .evaluate((element) => element.parentElement?.getBoundingClientRect().height);
  expect(paneHeight).toBeGreaterThan(boardBounds.height / 3);
});

test('alternating guard lists keep badges inside the diagram and expose every number', async ({ page }) => {
  const destinations = Array.from({ length: 80 }, (_, index) => (index % 2 === 0 ? 'closed' : 'other'));
  await openLifecycle(page, destinations);
  const diagram = page.getByRole('region', { name: 'Order lifecycle' });
  const badges = diagram.locator('svg g');
  const bounds = await diagram.boundingBox();
  if (!bounds) throw new Error('Lifecycle diagram has no bounds');
  await expect(badges).toHaveCount(2);
  for (let index = 0; index < 2; index++) {
    const badge = badges.nth(index);
    const badgeBounds = await badge.boundingBox();
    if (!badgeBounds) throw new Error('Transition badge has no bounds');
    expect(badgeBounds.x).toBeGreaterThanOrEqual(bounds.x);
    expect(badgeBounds.x + badgeBounds.width).toBeLessThanOrEqual(bounds.x + bounds.width);
    expect((await badge.locator('text').textContent())?.length).toBeLessThanOrEqual(24);
    await expect(badge.locator('title')).toHaveText(
      Array.from({ length: 40 }, (_, number) => number * 2 + index + 1).join(', '),
    );
  }
  await expect(page.getByTestId('state-machine-next').getByRole('listitem')).toHaveCount(80);
});

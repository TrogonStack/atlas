import { expect, test } from '@playwright/test';

// Fixture: one EventModel entity in the overview response.
const OVERVIEW_FIXTURE = {
  entities: [
    {
      eventModel: {
        id: { namespace: 'shop', slug: 'order-service', version: '1' },
        title: 'Order Service',
        doc: 'Core order lifecycle model.',
      },
    },
  ],
};

// Fixture: model entities for the EventModel page (empty members for simplicity).
const EVENT_MODEL_FIXTURE = {
  entities: [
    {
      eventModel: {
        id: { namespace: 'shop', slug: 'order-service', version: '1' },
        title: 'Order Service',
        doc: 'Core order lifecycle model.',
      },
    },
  ],
};

const INFO_FIXTURE = { schemaVersion: 'v1', serverVersion: 'test' };
const NAMESPACES_FIXTURE = { namespaces: ['shop'] };
const MODEL_FIXTURE = { entities: [] };

test.describe('Overview to board flow', () => {
  test.beforeEach(async ({ page }) => {
    // Intercept all API calls so the test never reaches a real gRPC backend.
    await page.route('/api/info', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(INFO_FIXTURE) }),
    );
    await page.route('/api/namespaces', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(NAMESPACES_FIXTURE) }),
    );
    await page.route('/api/overview', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(OVERVIEW_FIXTURE) }),
    );
    await page.route('/api/model**', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MODEL_FIXTURE) }),
    );
    await page.route('/api/event-model/**', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(EVENT_MODEL_FIXTURE) }),
    );
    // Abort realtime / NATS requests (not needed for this flow).
    await page.route('/api/changes**', (route) => route.abort());
  });

  test('Overview renders the model list and navigating to a model shows the board', async ({ page }) => {
    // Step 1: load the Overview landing page.
    await page.goto('/');

    // The "Overview" heading must appear (the landing page title).
    await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible();

    // The event model card should render with its title.
    await expect(page.getByText('Order Service')).toBeVisible();

    // The namespace registry is store-wide, so the overview is where it is linked from.
    const registryLink = page.getByRole('link', { name: 'Namespaces' });
    await expect(registryLink).toHaveAttribute('href', '/namespaces');

    // Step 2: click the event model card to navigate to the board.
    await page.getByRole('link', { name: /order service/i }).click();

    // The TopBar heading confirms we are on the EventModel page.
    await expect(page.getByRole('heading', { name: 'Event Model Studio', level: 1 })).toBeVisible();

    // The scoped title in the TopBar shows the model name.
    await expect(page.getByText('Order Service', { exact: true }).first()).toBeVisible();

    // A single model's top bar does not carry store-wide navigation.
    await expect(registryLink).toHaveCount(0);
  });

  test('the overview shows one view at a time, landing on the model list', async ({ page }) => {
    await page.goto('/');
    const modelsHeading = page.getByRole('heading', { name: /Event Models/ });
    const board = page.locator('.react-flow');

    await expect(modelsHeading).toBeVisible();
    await expect(page.getByRole('button', { name: 'Models', pressed: true })).toBeVisible();
    await expect(board).toHaveCount(0);

    await page.getByRole('button', { name: 'Domain chart' }).click();
    await expect(page).toHaveURL(/\?mode=chart$/);
    await expect(page.getByText('No domains mapped yet')).toBeVisible();
    await expect(modelsHeading).toHaveCount(0);

    await page.getByRole('button', { name: 'Contexts' }).click();
    await expect(page).toHaveURL(/\?mode=contexts$/);
    await expect(page.getByText('No bounded contexts yet')).toBeVisible();
    await expect(modelsHeading).toHaveCount(0);

    await page.getByRole('button', { name: 'Models' }).click();
    await expect(page).toHaveURL(/\/$/);
    await expect(modelsHeading).toBeVisible();
    await expect(board).toHaveCount(0);
  });

  test('the overview boards take real height, so their empty states stay inside them', async ({ page }) => {
    await page.goto('/?mode=chart');
    const search = page.getByRole('textbox', { name: 'Search models and entities' });
    const chartCard = page.getByText('No domains mapped yet');
    const chartLegend = page.getByText(/where investment goes/);
    await expect(chartCard).toBeVisible();

    const searchBox = await search.boundingBox();
    const cardBox = await chartCard.boundingBox();
    const legendBox = await chartLegend.boundingBox();
    if (!searchBox || !cardBox || !legendBox)
      throw new Error('Search box, empty state, or legend has no visible bounds');
    expect(cardBox.y).toBeGreaterThan(searchBox.y + searchBox.height);
    expect(legendBox.y).toBeGreaterThan(cardBox.y + cardBox.height);

    await page.getByRole('button', { name: 'Contexts' }).click();
    const contextsCard = page.getByText('No bounded contexts yet');
    const contextsLegend = page.getByText('published events into subscription views');
    await expect(contextsCard).toBeVisible();

    const contextsCardBox = await contextsCard.boundingBox();
    const contextsLegendBox = await contextsLegend.boundingBox();
    if (!contextsCardBox || !contextsLegendBox)
      throw new Error('Context map empty state or legend has no visible bounds');
    expect(contextsCardBox.y).toBeGreaterThan(searchBox.y + searchBox.height);
    expect(contextsLegendBox.y).toBeGreaterThan(contextsCardBox.y + contextsCardBox.height);
  });
});

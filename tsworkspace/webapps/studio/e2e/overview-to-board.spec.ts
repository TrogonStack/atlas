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

    // Step 2: click the event model card to navigate to the board.
    await page.getByRole('link', { name: /order service/i }).click();

    // The TopBar heading confirms we are on the EventModel page.
    await expect(page.getByRole('heading', { name: 'Event Model Studio', level: 1 })).toBeVisible();

    // The scoped title in the TopBar shows the model name.
    await expect(page.getByText('Order Service', { exact: true }).first()).toBeVisible();
  });
});

// Real-stack smoke suite: real Vite dev server, real Express bridge
// (server/index.mjs), real proto decoding, backed by the hermetic fake
// gRPC server started by e2e/fixtures/run-e2e-stack.mjs (see
// playwright.config.ts's webServer and e2e/README.md for why).
//
// This is the suite that would have caught the bug that motivated it: a
// stale in-tab ES module of shared/safe-namespace.mjs broke the app with
// "doesn't provide an export named isSafeBranchName" in a real browser,
// while all 294 vitest (jsdom) tests stayed green. jsdom never re-creates
// a real browser's module registry or reloads a page, so it cannot see
// this class of bug. Spec (a) below asserts zero console errors on first
// paint specifically to catch it and anything like it.
import { expect, test } from '@playwright/test';
import { FIXTURE_BRANCH, ORDER_EVENT_MODEL, ORDER_MODEL_ID } from './fixtures/fake-grpc-server.mjs';

const ORDER_TITLE = ORDER_EVENT_MODEL.eventModel.title;
const EM_PATH = `/em/${ORDER_MODEL_ID.namespace}/${ORDER_MODEL_ID.slug}`;

// Fails the test the moment a console error is observed, regardless of
// whether a later assertion would also have caught the symptom. Console
// warnings are left alone -- React and third-party libraries emit plenty
// of benign ones and gating on those would make this suite flaky for
// reasons unrelated to the app breaking.
function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&');
}

function failOnConsoleErrors(page: import('@playwright/test').Page, errors: string[]) {
  page.on('console', (msg) => {
    if (msg.type() === 'error') errors.push(msg.text());
  });
  page.on('pageerror', (err) => {
    errors.push(err.message);
  });
}

test.describe('Real-stack smoke', () => {
  test('(a) Overview page loads with zero console errors', async ({ page }) => {
    const errors: string[] = [];
    failOnConsoleErrors(page, errors);

    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible();
    await expect(page.getByText(ORDER_TITLE)).toBeVisible();

    expect(errors, `console errors on Overview load: ${JSON.stringify(errors)}`).toEqual([]);
  });

  test('(b) a model board URL loads and stickies render', async ({ page }) => {
    const errors: string[] = [];
    failOnConsoleErrors(page, errors);

    await page.goto(EM_PATH);
    await expect(page.getByRole('heading', { name: 'Event Model Studio', level: 1 })).toBeVisible();
    // The one Event member of the fixture EventModel renders as a sticky
    // with its title visible on the board.
    await expect(page.getByText('Order Placed')).toBeVisible();

    expect(errors, `console errors on board load: ${JSON.stringify(errors)}`).toEqual([]);
  });

  test('(c) ?branch= renders the BranchBar chip and read-only hint', async ({ page }) => {
    await page.goto(`${EM_PATH}?branch=${encodeURIComponent(FIXTURE_BRANCH.name)}`);
    await expect(page.getByRole('heading', { name: 'Event Model Studio', level: 1 })).toBeVisible();

    // The chip shows the branch name (font-mono span inside BranchBar).
    await expect(page.getByText(FIXTURE_BRANCH.name, { exact: true })).toBeVisible();
    // The read-only-preview hint is a fixed uppercase label next to the chip.
    await expect(page.getByText('read-only preview')).toBeVisible();
  });

  test('(d) the branch picker lists and switches branches', async ({ page }) => {
    await page.goto(EM_PATH);

    // No branch active yet: the toggle button reads "Branch".
    await page.getByRole('button', { name: 'Branch' }).click();
    await expect(page.getByText('No branches yet.')).not.toBeVisible();
    await expect(page.getByText(FIXTURE_BRANCH.doc)).toBeVisible();

    // Selecting the fixture branch switches the URL and the chip appears.
    await page.getByRole('button', { name: new RegExp(escapeRegExp(FIXTURE_BRANCH.name)) }).click();
    // nuqs does not percent-encode '/' in query values, so the URL carries
    // the branch name verbatim (e.g. "?branch=e2e/smoke-fixture").
    await expect(page).toHaveURL(new RegExp(escapeRegExp(`branch=${FIXTURE_BRANCH.name}`)));
    await expect(page.getByText(FIXTURE_BRANCH.name, { exact: true })).toBeVisible();

    // Switching back to baseline via the picker clears the chip.
    await page.getByRole('button', { name: 'Switch' }).click();
    await page.getByRole('button', { name: 'baseline' }).click();
    await expect(page).not.toHaveURL(/branch=/);
  });

  test('(e) the conflict drawer opens from the Review button with diff entries', async ({ page }) => {
    await page.goto(`${EM_PATH}?branch=${encodeURIComponent(FIXTURE_BRANCH.name)}`);
    await expect(page.getByText(FIXTURE_BRANCH.name, { exact: true })).toBeVisible();

    await page.getByRole('button', { name: 'Review' }).click();

    // ConflictInspector's header: "Branch review" badge + "Diff (<n>)" heading.
    await expect(page.getByText('Branch review')).toBeVisible();
    await expect(page.getByRole('heading', { name: /Diff \(\d+\)/ })).toBeVisible();
    // The fixture's single STATUS_CHANGED entry renders under the
    // "Changed" group heading in the left rail.
    await expect(page.getByRole('heading', { name: 'Changed', level: 3 })).toBeVisible();
  });
});

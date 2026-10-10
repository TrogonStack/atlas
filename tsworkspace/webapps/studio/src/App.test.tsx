import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { NuqsAdapter } from 'nuqs/adapters/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/components/OverviewPage', () => ({
  OverviewPage: () => <div data-testid="overview-page">Overview</div>,
}));
vi.mock('@/components/EventModelPage', () => ({
  EventModelPage: ({ namespace, slug }: { namespace: string; slug: string }) => (
    <div data-testid="em-page">
      {namespace}/{slug}
    </div>
  ),
}));

vi.mock('@/components/NamespacesPage', () => ({
  NamespacesPage: () => <div data-testid="namespaces-page">Namespaces</div>,
}));

import App from './App';

function renderApp() {
  return render(
    <NuqsAdapter>
      <App />
    </NuqsAdapter>,
  );
}

describe('App routing', () => {
  beforeEach(() => {
    window.history.replaceState(null, '', '/');
  });

  afterEach(() => {
    cleanup();
    window.history.replaceState(null, '', '/');
    vi.restoreAllMocks();
  });

  it('renders Overview on /', async () => {
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('overview-page')).toBeDefined();
    });
  });

  it('renders EventModelPage for /em/<namespace>/<slug>', async () => {
    window.history.replaceState(null, '', '/em/shop/checkout');
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('em-page')).toBeDefined();
    });
    expect(screen.getByText('shop/checkout')).toBeDefined();
  });

  it('decodes percent-encoded path segments', async () => {
    window.history.replaceState(null, '', '/em/my%2Dns/buy%2Dflow');
    renderApp();
    await waitFor(() => {
      expect(screen.getByText('my-ns/buy-flow')).toBeDefined();
    });
  });

  // BUG: decodeURIComponent throws URIError on malformed %xx; without a
  // try/catch the useState initializer crashes the first paint into the
  // ErrorBoundary. Falling back to Overview keeps routing total.
  it('falls back to Overview when a path segment is a malformed percent-encoding', async () => {
    window.history.replaceState(null, '', '/em/%ZZ/checkout');
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('overview-page')).toBeDefined();
    });
    expect(screen.queryByTestId('em-page')).toBeNull();
  });

  // BUG: decode succeeds for ".hidden", "foo;bar", etc., but those fail
  // isSafeNamespace. Routing them to EventModelPage only to show a 400
  // error screen is worse than falling back to Overview the same way
  // malformed percent-encoding already does.
  // Note: "%2e%2e" ("..") is a bad prove-case in jsdom/browsers; the
  // path is normalized away before parseLocation runs.
  it('falls back to Overview when a decoded path segment fails isSafeNamespace', async () => {
    window.history.replaceState(null, '', '/em/.hidden/checkout');
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('overview-page')).toBeDefined();
    });
    expect(screen.queryByTestId('em-page')).toBeNull();
  });

  it('falls back to Overview when a path segment contains shell metacharacters', async () => {
    window.history.replaceState(null, '', '/em/foo%3Bbar/checkout');
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('overview-page')).toBeDefined();
    });
    expect(screen.queryByTestId('em-page')).toBeNull();
  });

  it('renders NamespacesPage on /namespaces', async () => {
    window.history.replaceState(null, '', '/namespaces');
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('namespaces-page')).toBeDefined();
    });
  });

  it('accepts /namespaces with a trailing slash', async () => {
    window.history.replaceState(null, '', '/namespaces/');
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('namespaces-page')).toBeDefined();
    });
  });

  it('does not route a deeper path under /namespaces to the registry', async () => {
    window.history.replaceState(null, '', '/namespaces/acme');
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('overview-page')).toBeDefined();
    });
  });

  it('updates the route on popstate (browser back/forward)', async () => {
    window.history.replaceState(null, '', '/em/shop/checkout');
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('em-page')).toBeDefined();
    });

    window.history.pushState(null, '', '/');
    window.dispatchEvent(new PopStateEvent('popstate'));

    await waitFor(() => {
      expect(screen.getByTestId('overview-page')).toBeDefined();
    });
  });
});

describe('App credential prompt', () => {
  beforeEach(() => {
    window.history.replaceState(null, '', '/');
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('stays out of the way until something is refused', async () => {
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('overview-page')).toBeDefined();
    });
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('asks for a key when any request comes back unauthenticated', async () => {
    renderApp();
    await waitFor(() => {
      expect(screen.getByTestId('overview-page')).toBeDefined();
    });
    const { api } = await import('@/lib/api');
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ error: 'unauthenticated' }), {
          status: 401,
          statusText: 'Unauthorized',
          headers: { 'Content-Type': 'application/json' },
        }),
      ),
    );
    await expect(api.info()).rejects.toThrow();
    vi.unstubAllGlobals();
    await waitFor(() => {
      expect(screen.getByRole('dialog')).toBeDefined();
    });
    expect(screen.getByRole('alert').textContent).toContain('unauthenticated');
    // The page behind it is still mounted: the next fetch just carries the
    // new key, so there is nothing to reload.
    expect(screen.getByTestId('overview-page')).toBeDefined();
  });
});

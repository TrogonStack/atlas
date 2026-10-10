import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  api: {
    overview: vi.fn(),
    search: vi.fn(),
  },
}));

vi.mock('@/components/canvas/DomainChartBoard', () => ({
  DomainChartBoard: ({ onSelect }: { onSelect: (key: string) => void }) => (
    <button type="button" data-testid="domain-chart-board" onClick={() => onSelect('domain:market/trading@1')}>
      chart
    </button>
  ),
}));
vi.mock('@/components/canvas/ContextMapBoard', () => ({
  ContextMapBoard: () => <div data-testid="context-map-board" />,
}));
vi.mock('@xyflow/react', () => ({
  ReactFlowProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

import { api } from '@/lib/api';
import { OverviewPage } from './OverviewPage';

const EMPTY_OVERVIEW = { entities: [] };

const TWO_DOMAINS_OVERVIEW = {
  entities: [
    {
      domain: { id: { namespace: 'market', slug: 'trading', version: 1 }, title: 'Trading' },
    },
    {
      domain: { id: { namespace: 'market', slug: 'risk', version: 1 }, title: 'Risk' },
    },
  ],
};

// Simple in-memory localStorage stub to isolate test state.
function makeLocalStorageStub() {
  const store = new Map<string, string>();
  return {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => {
      store.set(key, value);
    },
    removeItem: (key: string) => {
      store.delete(key);
    },
    clear: () => {
      store.clear();
    },
    get length() {
      return store.size;
    },
    key: (index: number) => [...store.keys()][index] ?? null,
  };
}

describe('OverviewPage', () => {
  let localStorageStub: ReturnType<typeof makeLocalStorageStub>;

  beforeEach(() => {
    vi.clearAllMocks();
    localStorageStub = makeLocalStorageStub();
    vi.stubGlobal('localStorage', localStorageStub);
    // Reset the URL so state from a prior test's persistDomains call does not
    // leak into the next test's readUrlDomains() initialization.
    window.history.replaceState(null, '', '/');
    (api.overview as ReturnType<typeof vi.fn>).mockResolvedValue(EMPTY_OVERVIEW);
    (api.search as ReturnType<typeof vi.fn>).mockResolvedValue({ results: [] });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    cleanup();
  });

  describe('search debounce', () => {
    beforeEach(() => {
      vi.useFakeTimers();
    });

    it('does not fire a search before the debounce delay', () => {
      render(<OverviewPage />);
      const input = screen.getByPlaceholderText('Search models and entities');

      fireEvent.change(input, { target: { value: 'or' } });
      expect(api.search).not.toHaveBeenCalled();

      fireEvent.change(input, { target: { value: 'ord' } });
      expect(api.search).not.toHaveBeenCalled();
    });

    it('fires a search after the 200 ms debounce window', async () => {
      render(<OverviewPage />);
      const input = screen.getByPlaceholderText('Search models and entities');

      fireEvent.change(input, { target: { value: 'order' } });
      expect(api.search).not.toHaveBeenCalled();

      await act(async () => {
        await vi.runAllTimersAsync();
      });

      expect(api.search).toHaveBeenCalledWith(
        'order',
        undefined,
        expect.objectContaining({ signal: expect.any(AbortSignal) }),
      );
    });

    it('resets the debounce timer when the input changes quickly', async () => {
      render(<OverviewPage />);
      const input = screen.getByPlaceholderText('Search models and entities');

      fireEvent.change(input, { target: { value: 'ord' } });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(100);
      });

      fireEvent.change(input, { target: { value: 'order' } });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(100);
      });

      // First debounce was cancelled; still no call.
      expect(api.search).not.toHaveBeenCalled();

      await act(async () => {
        await vi.runAllTimersAsync();
      });

      expect(api.search).toHaveBeenCalledTimes(1);
      expect(api.search).toHaveBeenCalledWith('order', undefined, expect.anything());
    });

    it('clears search results when the query drops below 2 characters', async () => {
      (api.search as ReturnType<typeof vi.fn>).mockResolvedValue({
        results: [
          {
            entity: {
              event: {
                id: { namespace: 'shop', slug: 'order.placed', version: 1 },
                title: 'Order Placed',
              },
            },
          },
        ],
      });

      render(<OverviewPage />);
      const input = screen.getByPlaceholderText('Search models and entities');

      fireEvent.change(input, { target: { value: 'order' } });

      await act(async () => {
        await vi.runAllTimersAsync();
        await Promise.resolve();
      });

      expect(screen.getByText(/Search Results/)).toBeDefined();

      fireEvent.change(input, { target: { value: '' } });
      await act(async () => {
        await vi.runAllTimersAsync();
      });

      expect(screen.queryByText(/Search Results/)).toBeNull();
    });
  });

  describe('domain filter behavior', () => {
    // These tests use real timers; the overview fetch is resolved via act + flushPromises.
    async function renderWithDomains() {
      (api.overview as ReturnType<typeof vi.fn>).mockResolvedValue(TWO_DOMAINS_OVERVIEW);
      render(<OverviewPage />);
      // Flush the overview useEffect promise.
      await act(async () => {
        await new Promise<void>((resolve) => setTimeout(resolve, 0));
      });
    }

    it('shows domain filter buttons when multiple domains are loaded', async () => {
      await renderWithDomains();
      expect(screen.getByRole('button', { name: /trading/i })).toBeDefined();
      expect(screen.getByRole('button', { name: /risk/i })).toBeDefined();
    });

    it('activates domain filter and shows a clear button', async () => {
      await renderWithDomains();
      expect(screen.getByText(/showing all/i)).toBeDefined();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /trading/i }));
      });
      expect(screen.queryByText(/showing all/i)).toBeNull();
      expect(screen.getByRole('button', { name: /clear/i })).toBeDefined();
    });

    it('toggles a domain off on second click', async () => {
      await renderWithDomains();

      // First click: activates the filter; "clear" button should appear.
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /trading/i }));
      });
      // Clear button exists while at least one domain is selected.
      expect(screen.queryByRole('button', { name: /clear/i })).not.toBeNull();

      // Second click: deactivates the filter; "showing all" should return.
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /trading/i }));
      });
      expect(screen.getByText(/showing all/i)).toBeDefined();
    });

    it('clears all domain filters via the clear button', async () => {
      await renderWithDomains();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /trading/i }));
      });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /clear/i }));
      });

      expect(screen.getByText(/showing all/i)).toBeDefined();
    });
  });

  // BUG: Overview copy says "click a domain or context to read its charter",
  // but DomainChartBoard is wired with onSelect={() => {}}. Selecting a
  // domain therefore never surfaces charter details.
  it('surfaces a domain charter when a domain chart node is selected', async () => {
    (api.overview as ReturnType<typeof vi.fn>).mockResolvedValue({
      entities: [
        {
          domain: {
            id: { namespace: 'market', slug: 'trading', version: 1 },
            title: 'Trading',
            doc: 'The trading charter.',
          },
        },
        {
          domain: { id: { namespace: 'market', slug: 'risk', version: 1 }, title: 'Risk' },
        },
      ],
    });
    render(<OverviewPage />);
    await act(async () => {
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    fireEvent.click(screen.getByTestId('domain-chart-board'));
    expect(screen.getByText('The trading charter.')).toBeDefined();
  });
});

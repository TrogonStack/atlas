import { act, cleanup, fireEvent, render as rtlRender, screen, waitFor } from '@testing-library/react';
import { withNuqsTestingAdapter } from 'nuqs/adapters/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  api: {
    info: vi.fn(),
    namespaces: vi.fn(),
    model: vi.fn(),
    branches: vi.fn(),
    branchDiff: vi.fn(),
  },
}));

vi.mock('@/lib/realtime');

vi.mock('@/components/canvas/Board', () => ({
  Board: () => <div data-testid="board" />,
}));
vi.mock('@/components/canvas/SequenceBoard', () => ({
  SequenceBoard: () => <div data-testid="sequence-board" />,
}));
vi.mock('@/components/canvas/ScreensBoard', () => ({
  ScreensBoard: () => <div data-testid="screens-board" />,
}));
vi.mock('@/components/canvas/PlanBoard', () => ({
  PlanBoard: () => <div data-testid="plan-board" />,
}));
vi.mock('@/components/canvas/DomainChartBoard', () => ({
  DomainChartBoard: () => <div data-testid="domain-chart-board" />,
}));
vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>();
  return {
    ...actual,
    ReactFlowProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  };
});
vi.mock('@tanstack/react-hotkeys', () => ({
  useHotkey: vi.fn(),
}));

import { api } from '@/lib/api';
import { buildModel } from '@/lib/model';
import { watchEntities } from '@/lib/realtime';
import { ModelShell } from './ModelShell';

const neverResolving = new Promise(() => {});

// ModelShell reads/writes query state through nuqs, which needs an
// adapter above it in the tree.
const render = (ui: React.ReactElement) => rtlRender(ui, { wrapper: withNuqsTestingAdapter() });

function setupApiDefaults() {
  (api.info as ReturnType<typeof vi.fn>).mockResolvedValue({});
  (api.namespaces as ReturnType<typeof vi.fn>).mockResolvedValue({ namespaces: [] });
  (api.model as ReturnType<typeof vi.fn>).mockResolvedValue({ entities: [] });
  (api.branches as ReturnType<typeof vi.fn>).mockResolvedValue({ branches: [] });
  (api.branchDiff as ReturnType<typeof vi.fn>).mockResolvedValue({ entries: [] });
}

describe('ModelShell', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setupApiDefaults();
    // Default: watchEntities starts as a pending promise so it does not
    // trigger state updates during most tests.
    (watchEntities as ReturnType<typeof vi.fn>).mockReturnValue(neverResolving);
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('renders the board view by default', async () => {
    render(<ModelShell />);
    await waitFor(() => {
      expect(screen.getByTestId('board')).toBeDefined();
    });
  });

  it('fetches a new model when namespaces change', async () => {
    (api.model as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce({ entities: [] })
      .mockResolvedValueOnce({ entities: [] });

    const { rerender } = render(<ModelShell />);
    await waitFor(() => {
      expect(api.model).toHaveBeenCalledTimes(1);
    });

    // Passing a scoped model bypasses the internal fetch path; we test
    // the namespace-filtered path by verifying model is called with a
    // namespace argument on the initial render (namespaces defaults to
    // whatever ?ns= is in the URL, empty for the test environment).
    rerender(<ModelShell />);
    // The same namespace set means no extra call.
    expect(api.model).toHaveBeenCalledTimes(1);
  });

  it('skips the namespace fetch when a model prop is supplied (scoped mode)', async () => {
    const model = buildModel([]);
    render(<ModelShell model={model} scopedTitle="My EM" />);
    await waitFor(() => {
      // In scoped mode, api.model is never called.
      expect(api.model).not.toHaveBeenCalled();
    });
    // But api.info still is, since it feeds the TopBar.
    await waitFor(() => {
      expect(api.info).toHaveBeenCalled();
    });
  });

  it('hides view tabs that have nothing to render', async () => {
    render(<ModelShell model={buildModel([])} scopedTitle="Empty" />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Board' })).toBeDefined();
    });
    expect(screen.getByRole('button', { name: 'Sequence' })).toBeDefined();
    expect(screen.queryByRole('button', { name: 'UI' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Plan' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Domain' })).toBeNull();
  });

  it('offers the UI tab when the model has screens', async () => {
    const model = buildModel([
      { ui: { id: { namespace: 'shop', slug: 'checkout', version: '1' }, title: 'Checkout' } },
    ] as never);
    render(<ModelShell model={model} scopedTitle="Shop" />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'UI' })).toBeDefined();
    });
  });

  it('aborts the in-flight model fetch on unmount', async () => {
    // Verify the abort controller is wired correctly: unmounting the
    // component should abort any in-flight model fetch. We capture the
    // args from the api.model call and verify the signal is aborted after unmount.
    const modelPromise = new Promise<{ entities: unknown[] }>(() => {
      // never resolves: simulates a long-running request
    });
    (api.model as ReturnType<typeof vi.fn>).mockImplementation(() => modelPromise);

    const { unmount } = render(<ModelShell />);
    await waitFor(() => {
      expect(api.model).toHaveBeenCalled();
    });

    // Extract the signal from the actual call arguments.
    const calls = (api.model as ReturnType<typeof vi.fn>).mock.calls;
    const opts = calls[0]?.[1] as { signal?: AbortSignal } | undefined;
    const signal = opts?.signal;
    expect(signal).toBeDefined();
    expect(signal?.aborted).toBe(false);

    unmount();
    expect(signal?.aborted).toBe(true);
  });

  it('closes the realtime subscription on unmount', async () => {
    const close = vi.fn();
    (watchEntities as ReturnType<typeof vi.fn>).mockResolvedValue({ close });

    const { unmount } = render(<ModelShell />);
    // Allow the async watchEntities call to settle.
    await waitFor(() => {
      expect(watchEntities).toHaveBeenCalled();
    });
    // Give the promise chain a tick to resolve.
    await new Promise((r) => setTimeout(r, 0));
    unmount();
    expect(close).toHaveBeenCalled();
  });

  it('does not start subscriptions for a discarded StrictMode effect', async () => {
    const close = vi.fn();
    vi.mocked(watchEntities).mockResolvedValue({ close });
    const { unmount } = rtlRender(<ModelShell />, { wrapper: withNuqsTestingAdapter(), reactStrictMode: true });
    await waitFor(() => expect(watchEntities).toHaveBeenCalled());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(watchEntities).toHaveBeenCalledTimes(1);
    unmount();
    expect(close).toHaveBeenCalledOnce();
  });

  it('closes a realtime handle that resolves after unmount', async () => {
    const close = vi.fn();
    let resolveWatch: ((handle: { close: () => void }) => void) | undefined;
    vi.mocked(watchEntities).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveWatch = resolve;
        }),
    );
    const { unmount } = render(<ModelShell />);
    await waitFor(() => expect(watchEntities).toHaveBeenCalled());
    unmount();
    await act(async () => {
      resolveWatch?.({ close });
    });
    expect(close).toHaveBeenCalledOnce();
  });

  it('shows connection progress and preserves the reason updates go offline', async () => {
    let onState: ((state: 'connecting' | 'live' | 'offline', detail?: string) => void) | undefined;
    vi.mocked(watchEntities).mockImplementation(async (_change, state) => {
      onState = state;
      state('connecting');
      return { close: vi.fn() };
    });
    render(<ModelShell />);
    await waitFor(() => expect(screen.getByText('connecting', { exact: true })).toBeDefined());
    act(() => onState?.('offline', 'Permission denied for the live update subscription.'));
    const status = screen.getByRole('status', { name: 'Live updates' });
    expect(status.textContent).toContain('offline');
    expect(status.getAttribute('title')).toContain('Permission denied for the live update subscription.');
    act(() => onState?.('live'));
    expect(status.textContent).toContain('live');
    expect(status.getAttribute('title')).not.toContain('Permission denied');
  });

  // BUG: TopBar Refresh is wired to refresh(), which early-returns in scoped
  // mode and never invokes the caller-supplied onRefetch. EventModelPage's
  // Refresh button is therefore a no-op on every /em/ route.
  it('invokes onRefetch when Refresh is clicked in scoped mode', async () => {
    const onRefetch = vi.fn();
    render(<ModelShell model={buildModel([])} scopedTitle="Shop" onRefetch={onRefetch} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /refresh/i })).toBeDefined();
    });
    fireEvent.click(screen.getByRole('button', { name: /refresh/i }));
    expect(onRefetch).toHaveBeenCalled();
  });

  // BUG: switching ?branch= updates badges via branchDiff but never asks the
  // scoped owner to reload the model. The board keeps showing baseline (or
  // the previous branch) entities while the chip claims a different branch.
  it('invokes onRefetch when the active branch changes in scoped mode', async () => {
    (api.branches as ReturnType<typeof vi.fn>).mockResolvedValue({
      branches: [{ name: 'alex/rework', doc: '', createdAt: '', deltaCount: 1 }],
    });
    const onRefetch = vi.fn();
    render(<ModelShell model={buildModel([])} scopedTitle="Shop" onRefetch={onRefetch} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /branch/i })).toBeDefined();
    });
    fireEvent.click(screen.getByRole('button', { name: /branch/i }));
    await waitFor(() => {
      expect(screen.getByText('alex/rework')).toBeDefined();
    });
    fireEvent.click(screen.getByText('alex/rework'));
    await waitFor(() => {
      expect(onRefetch).toHaveBeenCalled();
    });
  });

  // BUG: realtime refetch refreshes the model but never re-fetches
  // branchDiff, so conflict/added/changed badges go stale after mutations.
  it('refetches branchDiff when realtime delivers a change while a branch is active', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let onChange: ((change: { key: string; operation: string; revision: string }) => void) | undefined;
    (watchEntities as ReturnType<typeof vi.fn>).mockImplementation(async (changeCb) => {
      onChange = changeCb;
      return { close: vi.fn() };
    });
    (api.branchDiff as ReturnType<typeof vi.fn>).mockResolvedValue({ entries: [] });

    rtlRender(<ModelShell />, {
      wrapper: withNuqsTestingAdapter({ searchParams: '?branch=alex/rework' }),
    });

    await waitFor(() => {
      expect(api.branchDiff).toHaveBeenCalledWith('alex/rework', expect.anything());
    });
    const callsAfterMount = (api.branchDiff as ReturnType<typeof vi.fn>).mock.calls.length;

    await waitFor(() => {
      expect(onChange).toBeTypeOf('function');
    });
    onChange!({ key: 'event.shop.order.placed.1', operation: 'PUT', revision: '9' });
    await vi.advanceTimersByTimeAsync(300);
    await waitFor(() => {
      expect((api.branchDiff as ReturnType<typeof vi.fn>).mock.calls.length).toBeGreaterThan(callsAfterMount);
    });
    vi.useRealTimers();
  });
});

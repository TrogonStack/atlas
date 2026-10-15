import { cleanup, fireEvent, render as rtlRender, screen, waitFor } from '@testing-library/react';
import { withNuqsTestingAdapter } from 'nuqs/adapters/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  api: {
    info: vi.fn(),
    namespaces: vi.fn(),
    model: vi.fn(),
    branches: vi.fn(),
    branchDiff: vi.fn(),
    changesets: vi.fn(),
    changeset: vi.fn(),
  },
}));
vi.mock('@/lib/realtime', () => ({
  watchEntities: vi.fn(() => new Promise(() => {})),
}));
vi.mock('@/components/canvas/Board', () => ({
  Board: ({
    focus,
    selections,
    onSelect,
  }: {
    focus?: { key: string; n: number };
    selections: unknown[];
    onSelect: (key: string) => void;
  }) => (
    <div data-testid="board" data-focus={JSON.stringify(focus)} data-selections={JSON.stringify(selections)}>
      <button type="button" onClick={() => onSelect('event:shop/order.placed@1')}>
        Board select order placed
      </button>
    </div>
  ),
}));
vi.mock('@/components/canvas/SequenceBoard', () => ({ SequenceBoard: () => null }));
vi.mock('@/components/canvas/ScreensBoard', () => ({ ScreensBoard: () => null }));
vi.mock('@/components/canvas/PlanBoard', () => ({ PlanBoard: () => null }));
vi.mock('@/components/canvas/DomainChartBoard', () => ({ DomainChartBoard: () => null }));
vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>();
  return { ...actual, ReactFlowProvider: ({ children }: { children: React.ReactNode }) => <>{children}</> };
});
vi.mock('@tanstack/react-hotkeys', () => ({ useHotkey: vi.fn() }));
vi.mock('@/components/Inspector', () => ({
  Inspector: ({
    entity,
    tab,
    onTabChange,
    onClose,
  }: {
    entity: { title: string };
    tab?: string;
    onTabChange?: (tab: string) => void;
    onClose: () => void;
  }) => (
    <div data-testid="inspector" data-tab={tab}>
      {entity.title}
      <button type="button" aria-label="Close inspector" onClick={onClose} />
      <button type="button" aria-label="Inspector tab details" onClick={() => onTabChange?.('details')} />
    </div>
  ),
}));
vi.mock('@/components/MultiInspector', () => ({
  MultiInspector: ({ entities }: { entities: unknown[] }) => <div data-testid="multi-inspector">{entities.length}</div>,
}));
vi.mock('@/components/EdgeInspector', () => ({
  EdgeInspector: () => <div data-testid="edge-inspector" />,
}));

import { api } from '@/lib/api';
import { buildIssueIndex } from '@/lib/issues';
import { buildModel } from '@/lib/model';
import { ModelShell } from './ModelShell';

describe('ModelShell selection drawer routing', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (api.info as ReturnType<typeof vi.fn>).mockResolvedValue({});
    (api.namespaces as ReturnType<typeof vi.fn>).mockResolvedValue({ namespaces: [] });
    (api.model as ReturnType<typeof vi.fn>).mockResolvedValue({ entities: [] });
    (api.branches as ReturnType<typeof vi.fn>).mockResolvedValue({ branches: [] });
    (api.branchDiff as ReturnType<typeof vi.fn>).mockResolvedValue({ entries: [] });
    (api.changesets as ReturnType<typeof vi.fn>).mockResolvedValue({ changesets: [], nextPageToken: '' });
    (api.changeset as ReturnType<typeof vi.fn>).mockResolvedValue({ changeset: null });
  });
  afterEach(cleanup);

  it('opens finding details and jumps to its entity using the shared focus request', async () => {
    const id = { namespace: 'shop', slug: 'order.placed', version: '1' };
    const model = buildModel([{ event: { id, title: 'Order placed' } }] as never);
    const issues = buildIssueIndex([
      {
        severity: 'SEVERITY_ERROR',
        code: 'MISSING_EVENT_FIELD',
        message: 'Order placed needs a customer identifier.',
        subject: { kind: 'ENTITY_KIND_EVENT', id },
        subjectField: 'schema.fields',
      },
    ]);
    rtlRender(<ModelShell model={model} scopedTitle="Shop" issues={issues} />, {
      wrapper: withNuqsTestingAdapter(),
    });

    fireEvent.click(screen.getByRole('button', { name: /validation findings/i }));
    fireEvent.click(screen.getByRole('button', { name: /shop\/order\.placed@1/ }));

    await waitFor(() => expect(screen.getByTestId('inspector').textContent).toBe('Order placed'));
    const board = screen.getByTestId('board');
    expect(JSON.parse(board.getAttribute('data-focus') ?? 'null')).toEqual({
      key: 'event:shop/order.placed@1',
      n: 1,
    });
    expect(JSON.parse(board.getAttribute('data-selections') ?? 'null')).toEqual([
      { type: 'entity', key: 'event:shop/order.placed@1' },
    ]);
    expect(screen.queryByRole('complementary', { name: /validation findings/i })).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Validation findings' }));
    fireEvent.click(screen.getByRole('button', { name: /shop\/order\.placed@1/ }));
    expect(JSON.parse(board.getAttribute('data-focus') ?? 'null')).toEqual({
      key: 'event:shop/order.placed@1',
      n: 2,
    });
  });

  it('replaces the previous selection when jumping to another entity from findings', async () => {
    const selectedId = { namespace: 'shop', slug: 'order.placed', version: '1' };
    const findingId = { namespace: 'shop', slug: 'order.shipped', version: '1' };
    const model = buildModel([
      { event: { id: selectedId, title: 'Order placed' } },
      { event: { id: findingId, title: 'Order shipped' } },
    ] as never);
    const issues = buildIssueIndex([
      {
        severity: 'SEVERITY_ERROR',
        code: 'MISSING_FIELD',
        message: 'A field is missing.',
        subject: { kind: 'ENTITY_KIND_EVENT', id: findingId },
      },
    ]);
    rtlRender(<ModelShell model={model} scopedTitle="Shop" issues={issues} />, {
      wrapper: withNuqsTestingAdapter({ searchParams: '?selected=event:shop/order.placed@1' }),
    });
    const board = screen.getByTestId('board');
    fireEvent.click(screen.getByRole('button', { name: 'Validation findings' }));
    fireEvent.click(screen.getByRole('button', { name: /shop\/order\.shipped@1/ }));
    await waitFor(() => expect(screen.getByTestId('inspector').textContent).toBe('Order shipped'));
    expect(JSON.parse(board.getAttribute('data-focus') ?? 'null')).toEqual({
      key: 'event:shop/order.shipped@1',
      n: 1,
    });
    expect(JSON.parse(board.getAttribute('data-selections') ?? 'null')).toEqual([
      { type: 'entity', key: 'event:shop/order.shipped@1' },
    ]);
    fireEvent.click(screen.getByRole('button', { name: 'Close inspector' }));
    await waitFor(() => expect(screen.queryByTestId('inspector')).toBeNull());
    expect(board.getAttribute('data-selections')).toBe('[]');
  });

  it('uses the right drawer slot for validation and restores the selected entity on close', () => {
    const id = { namespace: 'shop', slug: 'order.placed', version: '1' };
    const model = buildModel([{ event: { id, title: 'Order placed' } }] as never);
    const issues = buildIssueIndex([
      { severity: 'SEVERITY_ERROR', code: 'MISSING_FIELD', message: 'A field is missing.' },
    ]);
    rtlRender(<ModelShell model={model} scopedTitle="Shop" issues={issues} />, {
      wrapper: withNuqsTestingAdapter({ searchParams: `?selected=${encodeURIComponent(model.entities[0].key)}` }),
    });
    expect(screen.getByTestId('inspector')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Validation findings' }));
    const drawer = screen.getByRole('complementary', { name: 'Validation findings' });
    expect(drawer.parentElement).toBe(screen.getByRole('main').parentElement);
    expect(screen.getByRole('main').contains(drawer)).toBe(false);
    expect(screen.queryByTestId('inspector')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Close validation findings' }));
    expect(screen.queryByRole('complementary', { name: 'Validation findings' })).toBeNull();
    expect(screen.getByTestId('inspector')).toBeDefined();
  });

  it('opens Inspector (not MultiInspector) when one of two URL keys is stale', async () => {
    const model = buildModel([
      { event: { id: { namespace: 'shop', slug: 'order.placed', version: '1' }, title: 'Order placed' } },
    ] as never);
    const validKey = model.entities.find((e) => e.kind === 'event')!.key;
    rtlRender(<ModelShell model={model} scopedTitle="Shop" />, {
      wrapper: withNuqsTestingAdapter({
        searchParams: `?selected=${encodeURIComponent(validKey)},event:shop/ghost@1`,
      }),
    });
    await waitFor(() => {
      expect(screen.getByTestId('inspector')).toBeDefined();
    });
    expect(screen.queryByTestId('multi-inspector')).toBeNull();
    expect(screen.getByTestId('inspector').textContent).toBe('Order placed');
  });

  it('jumps the Inspector to the Validation tab from a finding, and resets to Details on a fresh selection', async () => {
    const id = { namespace: 'shop', slug: 'order.placed', version: '1' };
    const model = buildModel([{ event: { id, title: 'Order placed' } }] as never);
    const issues = buildIssueIndex([
      {
        severity: 'SEVERITY_ERROR',
        code: 'MISSING_EVENT_FIELD',
        message: 'Order placed needs a customer identifier.',
        subject: { kind: 'ENTITY_KIND_EVENT', id },
        subjectField: 'schema.fields',
      },
    ]);
    rtlRender(<ModelShell model={model} scopedTitle="Shop" issues={issues} />, {
      wrapper: withNuqsTestingAdapter(),
    });

    fireEvent.click(screen.getByRole('button', { name: /validation findings/i }));
    fireEvent.click(screen.getByRole('button', { name: /shop\/order\.placed@1/ }));
    await waitFor(() => expect(screen.getByTestId('inspector').getAttribute('data-tab')).toBe('validation'));

    // The Inspector reports its own tab changes back up; ModelShell must keep
    // its state in sync rather than only pushing `tab` one way down.
    fireEvent.click(screen.getByRole('button', { name: 'Inspector tab details' }));
    expect(screen.getByTestId('inspector').getAttribute('data-tab')).toBe('details');

    fireEvent.click(screen.getByRole('button', { name: 'Validation findings' }));
    fireEvent.click(screen.getByRole('button', { name: /shop\/order\.placed@1/ }));
    await waitFor(() => expect(screen.getByTestId('inspector').getAttribute('data-tab')).toBe('validation'));

    // A fresh selection from the board is a new look at an entity, not a
    // continuation of the validation jump: it must land back on Details.
    fireEvent.click(screen.getByRole('button', { name: 'Board select order placed' }));
    expect(screen.getByTestId('inspector').getAttribute('data-tab')).toBe('details');
  });

  it('opens namespace history from the top bar and jumps to an op on the Inspector History tab', async () => {
    const id = { namespace: 'shop', slug: 'order.placed', version: '1' };
    const model = buildModel([{ event: { id, title: 'Order placed' } }] as never);
    (api.changesets as ReturnType<typeof vi.fn>).mockResolvedValue({
      changesets: [
        {
          id: '0192f000-0000-7000-8000-00000000beef',
          author: 'acme-agent',
          at: '2026-01-02T00:00:00Z',
          message: 'put event shop/order.placed',
          branch: '',
          rpc: 'PutEntity',
          ops: [{ kind: 'KIND_PUT', entity: { kind: 'ENTITY_KIND_EVENT', ...id } }],
        },
      ],
      nextPageToken: '',
    });
    rtlRender(<ModelShell model={model} scopedTitle="Shop" historyNamespace="shop" />, {
      wrapper: withNuqsTestingAdapter(),
    });
    expect(api.changesets).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Namespace history' }));
    expect(screen.getByRole('complementary', { name: 'Namespace history' })).toBeDefined();
    fireEvent.click(await screen.findByRole('button', { name: /PutEntity/ }));
    fireEvent.click(screen.getByRole('button', { name: /shop\/order\.placed/ }));

    await waitFor(() => expect(screen.getByTestId('inspector').getAttribute('data-tab')).toBe('history'));
    expect(screen.queryByRole('complementary', { name: 'Namespace history' })).toBeNull();
    expect(JSON.parse(screen.getByTestId('board').getAttribute('data-selections') ?? 'null')).toEqual([
      { type: 'entity', key: 'event:shop/order.placed@1' },
    ]);
  });

  it('moves to a baseline changeset from history and exits from the as-of tag', async () => {
    const id = { namespace: 'shop', slug: 'order.placed', version: '1' };
    const model = buildModel([{ event: { id, title: 'Order placed' } }] as never);
    (api.changesets as ReturnType<typeof vi.fn>).mockResolvedValue({
      changesets: [
        {
          id: '0192f000-0000-7000-8000-00000000beef',
          author: 'acme-agent',
          at: '2026-01-02T00:00:00Z',
          message: 'put event shop/order.placed',
          branch: '',
          rpc: 'PutEntity',
          ops: [{ kind: 'KIND_PUT', entity: { kind: 'ENTITY_KIND_EVENT', ...id } }],
        },
      ],
      nextPageToken: '',
    });
    const onAsOfChange = vi.fn();
    const { rerender } = rtlRender(
      <ModelShell model={model} scopedTitle="Shop" historyNamespace="shop" onAsOfChange={onAsOfChange} />,
      { wrapper: withNuqsTestingAdapter() },
    );
    fireEvent.click(screen.getByRole('button', { name: 'Namespace history' }));
    fireEvent.click(await screen.findByRole('button', { name: /PutEntity/ }));
    fireEvent.click(screen.getByRole('button', { name: 'View board as of this' }));
    expect(onAsOfChange).toHaveBeenCalledWith('0192f000-0000-7000-8000-00000000beef');

    rerender(
      <ModelShell
        model={model}
        scopedTitle="Shop"
        historyNamespace="shop"
        asOf="0192f000-0000-7000-8000-00000000beef"
        onAsOfChange={onAsOfChange}
      />,
    );
    expect(screen.getByRole('status', { name: 'As of' })).toBeDefined();
    fireEvent.click(screen.getByRole('link', { name: 'Exit' }));
    expect(onAsOfChange).toHaveBeenLastCalledWith(undefined);
  });

  it('offers no history button without a namespace to read', () => {
    const model = buildModel([]);
    rtlRender(<ModelShell model={model} scopedTitle="Shop" />, { wrapper: withNuqsTestingAdapter() });
    expect(screen.queryByRole('button', { name: 'Namespace history' })).toBeNull();
  });
});

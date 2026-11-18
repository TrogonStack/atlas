// BUG proof: entity selections re-resolve from the live model (byKey), but
// edge selections snapshot BoardEdgeData at click time. After a refetch that
// updates edge doc/metadata (or endpoint titles), EdgeInspector stays stale.
import { cleanup, render as rtlRender, screen, waitFor } from '@testing-library/react';
import { withNuqsTestingAdapter } from 'nuqs/adapters/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BoardEdgeData } from '@/lib/layout';
import { buildModel, type Model } from '@/lib/model';
import { layoutBoards } from '@/lib/multiboard';

vi.mock('@/lib/api', () => ({
  api: {
    info: vi.fn(),
    namespaces: vi.fn(),
    model: vi.fn(),
    branches: vi.fn(),
    branchDiff: vi.fn(),
  },
}));
vi.mock('@/lib/realtime', () => ({
  watchEntities: vi.fn(() => new Promise(() => {})),
}));
vi.mock('@/components/canvas/Board', async () => {
  const actual = await vi.importActual<typeof import('@/components/canvas/Board')>('@/components/canvas/Board');
  return {
    ...actual,
    Board: ({
      onSelectEdge,
      model,
    }: {
      model: Model;
      onSelectEdge: (edge: { id: string; data: BoardEdgeData }, additive?: boolean) => void;
    }) => (
      <button
        type="button"
        data-testid="select-edge"
        onClick={() => {
          const persisted = layoutBoards(model).edges.find(
            (edge) => (edge.data as BoardEdgeData)?.connectionId?.value === 'occurrence-a',
          );
          if (persisted) {
            onSelectEdge({ id: persisted.id, data: persisted.data as BoardEdgeData });
            return;
          }
          onSelectEdge({
            id: 'cmd->ev',
            data: {
              relation: 'emits',
              doc: 'stale doc',
              metadata: [{ note: 'stale' }],
              source: {
                kind: 'command',
                id: { namespace: 'shop', slug: 'place', version: '1' },
                key: 'command:shop/place@1',
                title: 'Place (stale)',
                doc: '',
                annotations: [],
                fields: [],
                raw: {},
              },
              target: {
                kind: 'event',
                id: { namespace: 'shop', slug: 'placed', version: '1' },
                key: 'event:shop/placed@1',
                title: 'Placed (stale)',
                doc: '',
                annotations: [],
                fields: [],
                raw: {},
              },
            },
          });
        }}
      >
        select edge
      </button>
    ),
  };
});
vi.mock('@/components/canvas/SequenceBoard', () => ({ SequenceBoard: () => null }));
vi.mock('@/components/canvas/ScreensBoard', () => ({ ScreensBoard: () => null }));
vi.mock('@/components/canvas/PlanBoard', () => ({ PlanBoard: () => null }));
vi.mock('@/components/canvas/DomainChartBoard', () => ({ DomainChartBoard: () => null }));
vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>();
  return { ...actual, ReactFlowProvider: ({ children }: { children: React.ReactNode }) => <>{children}</> };
});
vi.mock('@tanstack/react-hotkeys', () => ({ useHotkey: vi.fn() }));
vi.mock('@/components/EdgeInspector', () => ({
  EdgeInspector: ({ edge }: { edge: BoardEdgeData }) => (
    <div data-testid="edge-inspector">
      <span data-testid="edge-id">{edge.connectionId?.value}</span>
      <span data-testid="edge-doc">{edge.doc}</span>
      <span data-testid="edge-meta">{JSON.stringify(edge.metadata)}</span>
      <span data-testid="edge-source">{edge.source.title}</span>
    </div>
  ),
}));

import { fireEvent } from '@testing-library/react';
import { api } from '@/lib/api';
import { ModelShell } from './ModelShell';

function fixture(doc: string, meta: unknown, sourceTitle: string) {
  return buildModel([
    {
      command: {
        id: { namespace: 'shop', slug: 'place', version: '1' },
        title: sourceTitle,
      },
    },
    {
      event: {
        id: { namespace: 'shop', slug: 'placed', version: '1' },
        title: 'Order placed',
      },
    },
    {
      commandSlice: {
        id: { namespace: 'shop', slug: 's-place', version: '1' },
        title: 'place order',
        command: { command: { id: { namespace: 'shop', slug: 'place', version: '1' } }, doc, metadata: meta },
        emittedEvents: [
          {
            event: { id: { namespace: 'shop', slug: 'placed', version: '1' } },
            doc,
            metadata: meta,
          },
        ],
      },
    },
  ] as never);
}

describe('BUG: EdgeInspector must refresh snapshotted edge data after model update', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (api.info as ReturnType<typeof vi.fn>).mockResolvedValue({});
    (api.namespaces as ReturnType<typeof vi.fn>).mockResolvedValue({ namespaces: [] });
    (api.model as ReturnType<typeof vi.fn>).mockResolvedValue({ entities: [] });
    (api.branches as ReturnType<typeof vi.fn>).mockResolvedValue({ branches: [] });
    (api.branchDiff as ReturnType<typeof vi.fn>).mockResolvedValue({ entries: [] });
  });
  afterEach(cleanup);

  it('shows updated edge doc/metadata/endpoint title when the scoped model prop refreshes', async () => {
    const initial = fixture('stale doc', [{ note: 'stale' }], 'Place (stale)');
    const { rerender } = rtlRender(<ModelShell model={initial} scopedTitle="Shop" />, {
      wrapper: withNuqsTestingAdapter({ searchParams: '?' }),
    });
    fireEvent.click(screen.getByTestId('select-edge'));
    await waitFor(() => expect(screen.getByTestId('edge-inspector')).toBeDefined());
    expect(screen.getByTestId('edge-doc').textContent).toBe('stale doc');

    const refreshed = fixture('fresh doc', [{ note: 'fresh' }], 'Place (fresh)');
    rerender(<ModelShell model={refreshed} scopedTitle="Shop" />);

    await waitFor(() => {
      expect(screen.getByTestId('edge-doc').textContent).toBe('fresh doc');
    });
    expect(screen.getByTestId('edge-meta').textContent).toContain('fresh');
    expect(screen.getByTestId('edge-source').textContent).toBe('Place (fresh)');
  });
  it('retains the selected persisted occurrence across equal-endpoint reorder and wording changes', async () => {
    const make = (reverse: boolean, uid = 'owner-incarnation') => {
      const model = fixture('same wording', [], 'Place');
      const owner = model.entities.find((entity) => entity.kind === 'commandSlice');
      if (!owner) throw new Error('Missing slice fixture');
      owner.uid = uid;
      const events = ['occurrence-a', 'occurrence-b'].map((value) => ({
        event: { id: { namespace: 'shop', slug: 'placed', version: '1' } },
        connectionId: { value },
        doc: reverse ? `updated ${value}` : 'same wording',
      }));
      owner.raw.emittedEvents = reverse ? events : events.reverse();
      return buildModel(model.entities.map((entity) => ({ [entity.kind]: entity.raw, system: { uid: entity.uid } })));
    };
    const { rerender } = rtlRender(<ModelShell model={make(false)} scopedTitle="Shop" />, {
      wrapper: withNuqsTestingAdapter({ searchParams: '?' }),
    });
    fireEvent.click(screen.getByTestId('select-edge'));
    await waitFor(() => expect(screen.getByTestId('edge-id').textContent).toBe('occurrence-a'));
    rerender(<ModelShell model={make(true)} scopedTitle="Shop" />);
    await waitFor(() => expect(screen.getByTestId('edge-doc').textContent).toBe('updated occurrence-a'));
    expect(screen.getByTestId('edge-id').textContent).toBe('occurrence-a');
    rerender(<ModelShell model={make(true, 'replacement-incarnation')} scopedTitle="Shop" />);
    await waitFor(() => expect(screen.queryByTestId('edge-inspector')).toBeNull());
  });
});

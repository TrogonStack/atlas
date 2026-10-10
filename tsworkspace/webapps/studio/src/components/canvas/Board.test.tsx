import { act, render, screen } from '@testing-library/react';
import { withNuqsTestingAdapter } from 'nuqs/adapters/testing';
import { type ReactNode, StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const flowProps = vi.hoisted(() => ({ current: undefined as Record<string, unknown> | undefined }));
const mockFitView = vi.hoisted(() => vi.fn());
const viewportState = vi.hoisted(() => ({ hasSavedViewport: false }));

vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>();
  return {
    ...actual,
    ReactFlow: (props: { children?: ReactNode }) => {
      flowProps.current = props as unknown as Record<string, unknown>;
      return <div data-testid="react-flow">{props.children}</div>;
    },
    ReactFlowProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
    useReactFlow: () => ({
      fitView: mockFitView,
      setCenter: vi.fn(),
      getNodes: vi.fn(() => []),
      getZoom: vi.fn(() => 1),
    }),
    Background: () => null,
    Controls: () => null,
    MiniMap: () => null,
  };
});

vi.mock('@/components/canvas/useFocusNode', () => ({
  useFocusNode: vi.fn(),
}));

vi.mock('@/components/canvas/usePersistedViewport', () => ({
  usePersistedViewport: (_key: string, enabled: boolean) => ({
    onMoveEnd: vi.fn(),
    restored: enabled && viewportState.hasSavedViewport,
  }),
}));

import { buildModel } from '@/lib/model';
import { Board } from './Board';
import { ContextMapBoard } from './ContextMapBoard';
import { DomainChartBoard } from './DomainChartBoard';
import { PlanBoard } from './PlanBoard';
import { ScreensBoard } from './ScreensBoard';
import { SequenceBoard } from './SequenceBoard';
import { kindStyle } from './StickyNode';

const emptyModel = buildModel([]);

describe('Board', () => {
  it('renders without crashing with an empty model', () => {
    render(<Board model={emptyModel} selections={[]} onSelect={vi.fn()} onSelectEdge={vi.fn()} />);
    expect(screen.getAllByTestId('react-flow').length).toBeGreaterThan(0);
  });

  it('keeps stickies non-draggable so positions stay owned by the layout engine', () => {
    render(<Board model={emptyModel} selections={[]} onSelect={vi.fn()} onSelectEdge={vi.fn()} />);
    expect(flowProps.current?.nodesDraggable).toBe(false);
    // Controlled React Flow still needs onNodesChange for its own changes
    // (measured dimensions, selection).
    expect(typeof flowProps.current?.onNodesChange).toBe('function');
  });
});

const wireId = (slug: string) => ({ namespace: 'demo', slug, version: '1' });

// Two disjoint event→read-model pairs: selecting one pair's read model
// must leave the pair lit and dim the other pair entirely.
const focusModel = buildModel([
  { event: { id: wireId('thing.published'), title: 'Thing published' } },
  {
    readModel: {
      id: wireId('things'),
      title: 'Things',
      sourceEvents: [{ id: wireId('thing.published') }],
    },
  },
  { event: { id: wireId('other.published'), title: 'Other published' } },
  {
    readModel: {
      id: wireId('others'),
      title: 'Others',
      sourceEvents: [{ id: wireId('other.published') }],
    },
  },
] as never);

type FlowNode = {
  id: string;
  type?: string;
  style?: { opacity?: number };
  data: { entity?: { key: string } };
};
type FlowEdge = { id: string; source: string; target: string; style?: { opacity?: number } };

function stickyByKey(key: string): FlowNode | undefined {
  const nodes = flowProps.current?.nodes as FlowNode[];
  return nodes.find((n) => n.type === 'sticky' && n.data.entity?.key === key);
}

describe('Board sticky focus', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mockFitView.mockClear();
    viewportState.hasSavedViewport = false;
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('dims stickies and connections not directly connected to the selected sticky', () => {
    render(
      <Board
        model={focusModel}
        selections={[{ type: 'entity', key: 'readModel:demo/things@1' }]}
        onSelect={vi.fn()}
        onSelectEdge={vi.fn()}
      />,
    );
    const selected = stickyByKey('readModel:demo/things@1');
    const neighbor = stickyByKey('event:demo/thing.published@1');
    const unrelatedEvent = stickyByKey('event:demo/other.published@1');
    const unrelatedRm = stickyByKey('readModel:demo/others@1');
    expect(selected?.style?.opacity).toBeUndefined();
    expect(neighbor?.style?.opacity).toBeUndefined();
    expect(unrelatedEvent?.style?.opacity).toBe(0.15);
    expect(unrelatedRm?.style?.opacity).toBe(0.15);

    const edges = flowProps.current?.edges as FlowEdge[];
    const connected = edges.filter((e) => e.source === neighbor?.id && e.target === selected?.id);
    const unrelated = edges.filter((e) => e.source === unrelatedEvent?.id);
    expect(connected.length).toBeGreaterThan(0);
    expect(unrelated.length).toBeGreaterThan(0);
    for (const e of connected) expect(e.style?.opacity).toBeUndefined();
    for (const e of unrelated) expect(e.style?.opacity).toBe(0.15);
  });

  it('keeps stickies undimmed for a connection-only selection', () => {
    const { unmount } = render(<Board model={focusModel} selections={[]} onSelect={vi.fn()} onSelectEdge={vi.fn()} />);
    const allEdges = flowProps.current?.edges as FlowEdge[];
    const target = allEdges[0];
    unmount();

    render(
      <Board
        model={focusModel}
        selections={[
          {
            type: 'edge',
            id: target.id,
            data: { relation: '', doc: '', metadata: [], source: undefined, target: undefined } as never,
          },
        ]}
        onSelect={vi.fn()}
        onSelectEdge={vi.fn()}
      />,
    );
    const nodes = flowProps.current?.nodes as FlowNode[];
    for (const n of nodes.filter((n) => n.type === 'sticky')) expect(n.style?.opacity).toBeUndefined();
    const edges = flowProps.current?.edges as FlowEdge[];
    for (const e of edges) {
      if (e.id === target.id) expect(e.style?.opacity).toBeUndefined();
      else expect(e.style?.opacity).toBe(0.15);
    }
  });

  it.each([false, true])('does not overwrite a new jump when a saved viewport exists: %s', (hasSavedViewport) => {
    viewportState.hasSavedViewport = hasSavedViewport;
    const props = { model: focusModel, selections: [], onSelect: vi.fn(), onSelectEdge: vi.fn() };
    const { rerender } = render(<Board {...props} />);
    rerender(<Board {...props} focus={{ key: 'readModel:demo/things@1', n: 1 }} />);
    act(() => {
      vi.runAllTimers();
    });
    expect(mockFitView).not.toHaveBeenCalled();
    expect(flowProps.current?.fitView).toBe(false);
  });

  it('preserves a resolved focus request when the board mounts', () => {
    render(
      <Board
        model={focusModel}
        selections={[]}
        onSelect={vi.fn()}
        onSelectEdge={vi.fn()}
        focus={{ key: 'readModel:demo/things@1', n: 1 }}
      />,
    );
    act(() => {
      vi.runAllTimers();
    });
    expect(mockFitView).not.toHaveBeenCalled();
    expect(flowProps.current?.fitView).toBe(false);
  });

  it('preserves the initial jump when StrictMode replays effects', () => {
    render(
      <StrictMode>
        <Board
          model={focusModel}
          selections={[]}
          onSelect={vi.fn()}
          onSelectEdge={vi.fn()}
          focus={{ key: 'readModel:demo/things@1', n: 1 }}
        />
      </StrictMode>,
    );
    act(() => {
      vi.runAllTimers();
    });
    expect(mockFitView).not.toHaveBeenCalled();
  });

  it('does not refit for an equivalent focus request without a graph change', () => {
    const props = { model: focusModel, selections: [], onSelect: vi.fn(), onSelectEdge: vi.fn() };
    const { rerender } = render(<Board {...props} focus={{ key: 'readModel:demo/things@1', n: 1 }} />);
    rerender(<Board {...props} focus={{ key: 'readModel:demo/things@1', n: 1 }} />);
    act(() => {
      vi.runAllTimers();
    });
    expect(mockFitView).not.toHaveBeenCalled();
  });

  it('does not refit after an equivalent model reload following a jump', () => {
    const focus = { key: 'readModel:demo/things@1', n: 1 };
    const props = { selections: [], onSelect: vi.fn(), onSelectEdge: vi.fn(), focus };
    const { rerender } = render(<Board {...props} model={focusModel} />);
    rerender(<Board {...props} model={{ ...focusModel, entities: [...focusModel.entities] }} />);
    act(() => {
      vi.runAllTimers();
    });
    expect(mockFitView).not.toHaveBeenCalled();
  });

  it('refits when the board graph changes after a completed (non-pending) focus', () => {
    // Comment in Board.tsx says only pending focus should suppress fitView;
    // a lingering FocusRequest from ModelShell must not permanently disable it.
    const modelA = focusModel;
    const modelB = buildModel([
      { event: { id: wireId('solo.published'), title: 'Solo published' } },
      {
        readModel: {
          id: wireId('solos'),
          title: 'Solos',
          sourceEvents: [{ id: wireId('solo.published') }],
        },
      },
    ] as never);

    const { rerender } = render(
      <Board
        model={modelA}
        selections={[]}
        onSelect={vi.fn()}
        onSelectEdge={vi.fn()}
        focus={{ key: 'readModel:demo/things@1', n: 1 }}
      />,
    );
    act(() => {
      vi.runAllTimers();
    });
    mockFitView.mockClear();

    rerender(
      <Board
        model={modelB}
        selections={[]}
        onSelect={vi.fn()}
        onSelectEdge={vi.fn()}
        focus={{ key: 'readModel:demo/things@1', n: 1 }}
      />,
    );
    act(() => {
      vi.runAllTimers();
    });
    expect(mockFitView).toHaveBeenCalled();
  });
});

describe('SequenceBoard', () => {
  it('renders without crashing with an empty model', () => {
    render(
      <SequenceBoard model={emptyModel} selections={[]} onSelect={vi.fn()} onSelectEdge={vi.fn()} />,
      // The ?rm= read-model filter lives in nuqs query state.
      { wrapper: withNuqsTestingAdapter() },
    );
    expect(screen.getAllByTestId('react-flow').length).toBeGreaterThan(0);
  });
});

describe('ScreensBoard', () => {
  it('renders without crashing with an empty model', () => {
    render(<ScreensBoard model={emptyModel} selections={[]} onSelect={vi.fn()} onSelectEdge={vi.fn()} />);
    expect(screen.getAllByTestId('react-flow').length).toBeGreaterThan(0);
  });
});

describe('PlanBoard', () => {
  it('renders without crashing with an empty model', () => {
    render(<PlanBoard model={emptyModel} selections={[]} onSelect={vi.fn()} />);
    expect(screen.getAllByTestId('react-flow').length).toBeGreaterThan(0);
  });

  it('keeps artifacts non-draggable so positions stay owned by the layout engine', () => {
    render(<PlanBoard model={emptyModel} selections={[]} onSelect={vi.fn()} />);
    expect(flowProps.current?.nodesDraggable).toBe(false);
    expect(typeof flowProps.current?.onNodesChange).toBe('function');
  });
});

describe('DomainChartBoard', () => {
  it('renders without crashing with an empty model', () => {
    render(<DomainChartBoard model={emptyModel} selections={[]} onSelect={vi.fn()} />);
    expect(screen.getAllByTestId('react-flow').length).toBeGreaterThan(0);
  });
});

describe('ContextMapBoard', () => {
  it('renders without crashing with an empty model', () => {
    render(<ContextMapBoard model={emptyModel} selections={[]} onSelect={vi.fn()} />);
    expect(screen.getAllByTestId('react-flow').length).toBeGreaterThan(0);
  });
});

describe('kindStyle', () => {
  it('returns a fallback for unknown kinds', () => {
    const style = kindStyle('unknownKind' as never);
    expect(style.label).toBe('unknownKind');
  });
});

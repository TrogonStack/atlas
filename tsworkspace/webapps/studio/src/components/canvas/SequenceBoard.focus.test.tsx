import { render } from '@testing-library/react';
import { withNuqsTestingAdapter } from 'nuqs/adapters/testing';
import type React from 'react';
import { describe, expect, it, vi } from 'vitest';

const flowProps = vi.hoisted(() => ({ current: undefined as Record<string, unknown> | undefined }));

vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>();
  return {
    ...actual,
    ReactFlow: (props: { children?: React.ReactNode }) => {
      flowProps.current = props as unknown as Record<string, unknown>;
      return <div data-testid="react-flow">{props.children}</div>;
    },
    ReactFlowProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
    useReactFlow: () => ({
      fitView: vi.fn(),
      setCenter: vi.fn(),
      getNodes: vi.fn(() => []),
      getZoom: vi.fn(() => 1),
    }),
    Background: () => null,
    Controls: () => null,
  };
});

vi.mock('@/components/canvas/useFocusNode', () => ({
  useFocusNode: vi.fn(),
}));

import { buildModel } from '@/lib/model';
import { SequenceBoard } from './SequenceBoard';

const wireId = (slug: string) => ({ namespace: 'demo', slug, version: '1' });

// Two disjoint event→read-model pairs on a sequence canvas: selecting one
// sticky should light its neighbor edge the same way Board.focusSets does.
const seqModel = buildModel([
  { event: { id: wireId('thing.published'), title: 'Thing published' } },
  {
    readModel: {
      id: wireId('things'),
      title: 'Things',
      sourceEvents: [{ id: wireId('thing.published') }],
    },
  },
  {
    readModelSlice: {
      id: wireId('s-things'),
      title: 'things',
      readModel: { readModel: { id: wireId('things') } },
      sourceEvents: [{ event: { id: wireId('thing.published') } }],
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
  {
    readModelSlice: {
      id: wireId('s-others'),
      title: 'others',
      readModel: { readModel: { id: wireId('others') } },
      sourceEvents: [{ event: { id: wireId('other.published') } }],
    },
  },
  {
    storyboard: {
      id: wireId('sb'),
      title: 'sb',
      slices: [{ id: wireId('s-things') }, { id: wireId('s-others') }],
    },
  },
] as never);

type FlowNode = {
  id: string;
  type?: string;
  style?: { opacity?: number };
  data: { entity?: { key: string }; selected?: boolean };
};
type FlowEdge = { id: string; source: string; target: string; style?: { opacity?: number } };

describe('SequenceBoard sticky focus parity with Board', () => {
  it('dims stickies and connections not directly connected to the selected sticky', () => {
    render(
      <SequenceBoard
        model={seqModel}
        selections={[{ type: 'entity', key: 'readModel:demo/things@1' }]}
        onSelect={vi.fn()}
        onSelectEdge={vi.fn()}
      />,
      { wrapper: withNuqsTestingAdapter() },
    );

    const nodes = (flowProps.current?.nodes as FlowNode[]).filter((n) => n.type === 'seqSticky');
    const edges = flowProps.current?.edges as FlowEdge[];
    const selected = nodes.find((n) => n.data.entity?.key === 'readModel:demo/things@1');
    const unrelated = nodes.find((n) => n.data.entity?.key === 'readModel:demo/others@1');

    expect(selected, `stickies=${nodes.map((n) => n.data.entity?.key).join(',')}`).toBeTruthy();
    expect(unrelated).toBeTruthy();
    // Board.focusSets dims non-adjacent stickies; SequenceBoard should match.
    expect(selected?.style?.opacity).toBeUndefined();
    expect(unrelated?.style?.opacity).toBe(0.15);

    const touching = edges.filter((e) => e.source === selected?.id || e.target === selected?.id);
    const notTouching = edges.filter((e) => e.source !== selected?.id && e.target !== selected?.id);
    expect(touching.length + notTouching.length).toBe(edges.length);
    if (notTouching.length > 0) {
      expect(notTouching.some((e) => e.style?.opacity === 0.15)).toBe(true);
    }
  });
});

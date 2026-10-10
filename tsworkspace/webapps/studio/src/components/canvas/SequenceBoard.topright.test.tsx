// The sequence board's read-model filter used to anchor itself at
// `absolute top-3 right-3`, the same corner ModelShell anchors the validator
// summary to, so the filter rendered on top of the summary's counts. It now
// goes through BoardTopRight; this holds it there.

import { render, screen } from '@testing-library/react';
import { withNuqsTestingAdapter } from 'nuqs/adapters/testing';
import type React from 'react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>();
  return {
    ...actual,
    ReactFlow: (props: { children?: React.ReactNode }) => <div data-testid="react-flow">{props.children}</div>,
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
import { BoardTopRightHost } from './BoardTopRight';
import { SequenceBoard } from './SequenceBoard';

const wireId = (slug: string) => ({ namespace: 'demo', slug, version: '1' });

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
  {
    storyboard: {
      id: wireId('sb'),
      title: 'sb',
      slices: [{ id: wireId('s-things') }],
    },
  },
] as never);

describe('SequenceBoard top-right control', () => {
  it('shares the shell corner instead of anchoring on top of it', () => {
    render(
      <BoardTopRightHost corner={<span>validation summary</span>}>
        <SequenceBoard model={seqModel} selections={[]} onSelect={vi.fn()} onSelectEdge={vi.fn()} />
      </BoardTopRightHost>,
      { wrapper: withNuqsTestingAdapter() },
    );

    const row = screen.getByTestId('board-top-right');
    const filter = screen.getByRole('combobox');
    expect(row.contains(filter)).toBe(true);
    expect(row.contains(screen.getByText('validation summary'))).toBe(true);
    // Nothing inside the row may re-anchor itself to the corner.
    expect(row.querySelector('.absolute')).toBeNull();
  });
});

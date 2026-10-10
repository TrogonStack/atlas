import { describe, expect, it } from 'vitest';
import { buildModel } from './model';
import { layoutBoards } from './multiboard';

const id = (namespace: string, slug: string): Record<string, unknown> => ({
  namespace,
  slug,
  version: '1',
});

describe('layoutBoards', () => {
  it('does not render halo-only partitions (borrowed source events)', () => {
    const model = buildModel([
      { event: { id: id('upstream', 'thing.published'), title: 'Thing published' } },
      {
        readModel: {
          id: id('downstream', 'thing-feed'),
          title: 'Thing feed',
          sourceEvents: [{ id: id('upstream', 'thing.published') }],
        },
      },
    ] as never);

    const { nodes, edges } = layoutBoards(model);
    // Nothing from the upstream namespace lands on the board: no context
    // band, no borrowed event card, no seam arrow.
    expect(nodes.some((n) => n.id === 'ctx:upstream' || n.id.startsWith('upstream::'))).toBe(false);
    expect(edges.some((e) => e.id.startsWith('seam:'))).toBe(false);
    // The downstream context keeps its full board.
    expect(nodes.some((n) => n.id === 'ctx:downstream')).toBe(true);
    expect(nodes.some((n) => n.id.startsWith('downstream::') && n.type === 'lane')).toBe(true);
  });

  it('keeps real contexts (with their own behavior) as full groups', () => {
    const model = buildModel([
      { event: { id: id('upstream', 'thing.published'), title: 'Thing published' } },
      {
        readModel: {
          id: id('upstream', 'things'),
          title: 'Things',
          sourceEvents: [{ id: id('upstream', 'thing.published') }],
        },
      },
      {
        readModel: {
          id: id('downstream', 'thing-feed'),
          title: 'Thing feed',
          sourceEvents: [{ id: id('upstream', 'thing.published') }],
        },
      },
    ] as never);

    const { nodes, edges } = layoutBoards(model);
    expect(nodes.some((n) => n.id === 'ctx:upstream')).toBe(true);
    expect(nodes.some((n) => n.id === 'ctx:downstream')).toBe(true);
    // With the upstream event actually on the board, the seam still draws.
    expect(edges.some((e) => e.id.startsWith('seam:'))).toBe(true);
  });
});

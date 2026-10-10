import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mockSetCenter = vi.fn();
const mockGetZoom = vi.fn(() => 1);

vi.mock('@xyflow/react', () => ({
  useReactFlow: () => ({ setCenter: mockSetCenter, getZoom: mockGetZoom }),
}));

import type { Node } from '@xyflow/react';
import type { FocusRequest } from './useFocusNode';
import { useFocusNode } from './useFocusNode';

function makeNode(id: string, key: string, x = 0, y = 0, width = 216, height = 122, type = 'sticky'): Node {
  return {
    id,
    type,
    position: { x, y },
    width,
    height,
    data: { entity: { key } },
  };
}

describe('useFocusNode', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetZoom.mockReturnValue(1);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('does not call setCenter when focus is undefined', () => {
    renderHook(() => useFocusNode(undefined, []));
    expect(mockSetCenter).not.toHaveBeenCalled();
  });

  it('does not call setCenter when focus is pending', () => {
    const focus: FocusRequest = { pending: true, n: 1 };
    renderHook(() => useFocusNode(focus, []));
    expect(mockSetCenter).not.toHaveBeenCalled();
  });

  it('does not call setCenter when no matching node is found', () => {
    const focus: FocusRequest = { key: 'missing', n: 1 };
    const nodes = [makeNode('node-other', 'other')];
    renderHook(() => useFocusNode(focus, nodes));
    expect(mockSetCenter).not.toHaveBeenCalled();
  });

  it('calls setCenter centered on the matched node', () => {
    const focus: FocusRequest = { key: 'order-placed', n: 1 };
    const nodes = [makeNode('node-1', 'order-placed', 100, 200, 216, 122)];
    renderHook(() => useFocusNode(focus, nodes));
    expect(mockSetCenter).toHaveBeenCalledWith(
      100 + 216 / 2,
      200 + 122 / 2,
      expect.objectContaining({ duration: 600 }),
    );
  });

  it.each([
    [0.2, 0.8],
    [0.9, 0.9],
    [2, 1],
  ])('centers with a moderate zoom when the current zoom is %s', (currentZoom, expectedZoom) => {
    mockGetZoom.mockReturnValue(currentZoom);
    renderHook(() => useFocusNode({ key: 'order-placed', n: 1 }, [makeNode('node-1', 'order-placed')]));
    expect(mockSetCenter).toHaveBeenCalledWith(108, 61, { duration: 600, zoom: expectedZoom });
  });

  it('prefers the node matching preferType when multiple nodes share the same entity key', () => {
    const focus: FocusRequest = { key: 'shared', n: 1 };
    const nodes = [
      makeNode('node-a', 'shared', 0, 0, 216, 122, 'sticky'),
      makeNode('node-b', 'shared', 500, 500, 216, 122, 'special'),
    ];
    renderHook(() => useFocusNode(focus, nodes, 'special'));
    const [cx, cy] = mockSetCenter.mock.calls[0] as [number, number];
    expect(cx).toBe(500 + 216 / 2);
    expect(cy).toBe(500 + 122 / 2);
  });

  it('recenters when focus reference changes', async () => {
    const nodes = [makeNode('n-e1', 'e1', 0, 0), makeNode('n-e2', 'e2', 300, 300)];
    const initialFocus: FocusRequest = { key: 'e1', n: 1 };
    const { rerender } = renderHook(({ focus }: { focus: FocusRequest }) => useFocusNode(focus, nodes), {
      initialProps: { focus: initialFocus },
    });
    expect(mockSetCenter).toHaveBeenCalledTimes(1);

    const nextFocus: FocusRequest = { key: 'e2', n: 2 };
    await act(async () => {
      rerender({ focus: nextFocus });
    });
    expect(mockSetCenter).toHaveBeenCalledTimes(2);
    const [cx, cy] = mockSetCenter.mock.calls[1] as [number, number];
    expect(cx).toBe(300 + 216 / 2);
    expect(cy).toBe(300 + 122 / 2);
  });

  it('does not re-pan when only the nodes array identity changes (focus.n unchanged)', async () => {
    // focus.n is the intentional re-trigger; layout refreshes must not yank
    // the camera after the user has panned away.
    const focus: FocusRequest = { key: 'e1', n: 1 };
    const nodesA = [makeNode('n-e1', 'e1', 0, 0)];
    const { rerender } = renderHook(({ nodes }: { nodes: Node[] }) => useFocusNode(focus, nodes), {
      initialProps: { nodes: nodesA },
    });
    expect(mockSetCenter).toHaveBeenCalledTimes(1);

    const nodesB = [makeNode('n-e1', 'e1', 0, 0)];
    await act(async () => {
      rerender({ nodes: nodesB });
    });
    expect(mockSetCenter).toHaveBeenCalledTimes(1);
  });
});

import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mockSetViewport = vi.fn();

vi.mock('@xyflow/react', () => ({
  useReactFlow: () => ({ setViewport: mockSetViewport }),
}));

import { usePersistedViewport } from './usePersistedViewport';

const STORAGE_KEY = 'trogon-atlas-studio:viewport:test-board';

function makeStorage() {
  const store = new Map<string, string>();
  return {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => {
      store.set(k, v);
    },
    removeItem: (k: string) => {
      store.delete(k);
    },
    clear: () => {
      store.clear();
    },
    get length() {
      return store.size;
    },
    key: (i: number) => [...store.keys()][i] ?? null,
  };
}

describe('usePersistedViewport', () => {
  let storageStub: ReturnType<typeof makeStorage>;

  beforeEach(() => {
    vi.clearAllMocks();
    storageStub = makeStorage();
    vi.stubGlobal('sessionStorage', storageStub);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('returns restored=false when there is no saved viewport', () => {
    const { result } = renderHook(() => usePersistedViewport('test-board', true));
    expect(result.current.restored).toBe(false);
  });

  it('returns restored=true when a valid viewport is already in sessionStorage', () => {
    storageStub.setItem(STORAGE_KEY, JSON.stringify({ x: 10, y: 20, zoom: 1.5 }));
    const { result } = renderHook(() => usePersistedViewport('test-board', true));
    expect(result.current.restored).toBe(true);
  });

  it('calls setViewport with the persisted values after the timer fires', async () => {
    vi.useFakeTimers();
    storageStub.setItem(STORAGE_KEY, JSON.stringify({ x: 10, y: 20, zoom: 1.5 }));
    renderHook(() => usePersistedViewport('test-board', true));

    await act(async () => {
      vi.runAllTimers();
    });

    expect(mockSetViewport).toHaveBeenCalledWith({ x: 10, y: 20, zoom: 1.5 }, { duration: 0 });
  });

  it('persists the viewport to sessionStorage via onMoveEnd', () => {
    const { result } = renderHook(() => usePersistedViewport('test-board', true));
    act(() => {
      result.current.onMoveEnd(null, { x: 5, y: 15, zoom: 0.8 });
    });
    const stored = storageStub.getItem(STORAGE_KEY);
    expect(stored).toBe(JSON.stringify({ x: 5, y: 15, zoom: 0.8 }));
  });

  it('does not persist via onMoveEnd when disabled', () => {
    const { result } = renderHook(() => usePersistedViewport('test-board', false));
    act(() => {
      result.current.onMoveEnd(null, { x: 5, y: 15, zoom: 0.8 });
    });
    expect(storageStub.getItem(STORAGE_KEY)).toBeNull();
  });

  it('returns restored=false when disabled regardless of stored data', () => {
    storageStub.setItem(STORAGE_KEY, JSON.stringify({ x: 10, y: 20, zoom: 1 }));
    const { result } = renderHook(() => usePersistedViewport('test-board', false));
    expect(result.current.restored).toBe(false);
  });

  it('reports restored=true on the first render after the key changes to one with a saved viewport', () => {
    // Board reads `restored` during render for fitView={!restored}. A stale
    // false on that commit lets React Flow mount with fitView before the
    // effect's setViewport runs.
    storageStub.setItem('trogon-atlas-studio:viewport:board-b', JSON.stringify({ x: 40, y: 50, zoom: 1.2 }));
    const restoredByRender: boolean[] = [];
    const { rerender } = renderHook(
      ({ key }: { key: string }) => {
        const result = usePersistedViewport(key, true);
        restoredByRender.push(result.restored);
        return result;
      },
      { initialProps: { key: 'board-a' } },
    );
    restoredByRender.length = 0;

    rerender({ key: 'board-b' });
    expect(restoredByRender[0]).toBe(true);
  });
});

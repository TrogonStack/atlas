import { act, cleanup, renderHook } from '@testing-library/react';
import { createElement, useLayoutEffect } from 'react';
import { renderToString } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetFeatureFlagsForTests, useStudioFeatureFlags } from './feature-flags';

const STORAGE_KEY = 'trogon-atlas.studio.feature-flags';

function changedStorage(key: string | null = STORAGE_KEY, storageArea = window.localStorage) {
  window.dispatchEvent(new StorageEvent('storage', { key, storageArea }));
}

describe('Studio feature flags', () => {
  beforeEach(() => {
    window.localStorage.clear();
    resetFeatureFlagsForTests();
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    resetFeatureFlagsForTests();
  });

  it('starts disabled and preserves independent choices across remounts', () => {
    const { result, unmount } = renderHook(useStudioFeatureFlags);
    expect(result.current[0]).toEqual({ laneMesh: false, streamTowers: false, stateMachine: false });
    act(() => result.current[1]('laneMesh', true));
    act(() => result.current[1]('streamTowers', true));
    act(() => result.current[1]('laneMesh', false));
    unmount();

    const reloaded = renderHook(useStudioFeatureFlags);
    expect(reloaded.result.current[0]).toEqual({ laneMesh: false, streamTowers: true, stateMachine: false });
  });

  it.each(['broken', 'null', '[]', 'true', '{"laneMesh":"true","streamTowers":1}'])(
    'keeps malformed stored flags disabled: %s',
    (stored) => {
      window.localStorage.setItem(STORAGE_KEY, stored);
      const { result } = renderHook(useStudioFeatureFlags);
      expect(result.current[0]).toEqual({ laneMesh: false, streamTowers: false, stateMachine: false });
    },
  );

  it.each([
    ['{"laneMesh":true,"unknown":true}', { laneMesh: true, streamTowers: false, stateMachine: false }],
    ['{"laneMesh":true,"streamTowers":"true"}', { laneMesh: true, streamTowers: false, stateMachine: false }],
    ['{"laneMesh":1,"streamTowers":true}', { laneMesh: false, streamTowers: true, stateMachine: false }],
  ] as const)('accepts only explicitly enabled known flags from %s', (stored, expected) => {
    window.localStorage.setItem(STORAGE_KEY, stored);
    const { result } = renderHook(useStudioFeatureFlags);
    expect(result.current[0]).toEqual(expected);
  });

  it('keeps separate consumers synchronized and merges changes from a stale consumer', () => {
    const first = renderHook(useStudioFeatureFlags);
    const second = renderHook(useStudioFeatureFlags);
    const secondSetter = second.result.current[1];

    act(() => first.result.current[1]('laneMesh', true));
    expect(second.result.current[0]).toEqual({ laneMesh: true, streamTowers: false, stateMachine: false });
    act(() => secondSetter('streamTowers', true));
    expect(first.result.current[0]).toEqual({ laneMesh: true, streamTowers: true, stateMachine: false });
    expect(second.result.current[0]).toBe(first.result.current[0]);
    expect(JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? 'null')).toEqual({
      laneMesh: true,
      streamTowers: true,
      stateMachine: false,
    });
  });

  it('merges the latest persisted value before writing another flag', () => {
    const { result } = renderHook(useStudioFeatureFlags);
    window.localStorage.setItem(STORAGE_KEY, '{"laneMesh":true,"streamTowers":false}');

    act(() => result.current[1]('streamTowers', true));

    expect(result.current[0]).toEqual({ laneMesh: true, streamTowers: true, stateMachine: false });
    expect(JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? 'null')).toEqual({
      laneMesh: true,
      streamTowers: true,
      stateMachine: false,
    });
  });

  it('updates mounted consumers from localStorage events', () => {
    const { result } = renderHook(useStudioFeatureFlags);
    act(() => {
      window.localStorage.setItem(STORAGE_KEY, '{"laneMesh":false,"streamTowers":true}');
      changedStorage();
    });
    expect(result.current[0]).toEqual({ laneMesh: false, streamTowers: true, stateMachine: false });
  });

  it.each(['remove', 'clear'] as const)('resets mounted flags after an external %s', (operation) => {
    window.localStorage.setItem(STORAGE_KEY, '{"laneMesh":true,"streamTowers":true}');
    const { result } = renderHook(useStudioFeatureFlags);
    act(() => {
      if (operation === 'remove') window.localStorage.removeItem(STORAGE_KEY);
      else window.localStorage.clear();
      changedStorage(operation === 'clear' ? null : STORAGE_KEY);
    });
    expect(result.current[0]).toEqual({ laneMesh: false, streamTowers: false, stateMachine: false });
  });

  it('ignores unrelated keys and sessionStorage events', () => {
    const { result } = renderHook(useStudioFeatureFlags);
    const snapshot = result.current[0];
    act(() => {
      window.localStorage.setItem('unrelated-setting', 'true');
      changedStorage('unrelated-setting');
      window.sessionStorage.setItem(STORAGE_KEY, '{"laneMesh":true,"streamTowers":true}');
      changedStorage(STORAGE_KEY, window.sessionStorage);
    });
    expect(result.current[0]).toBe(snapshot);
    expect(result.current[0]).toEqual({ laneMesh: false, streamTowers: false, stateMachine: false });
  });

  it('keeps snapshot and setter identities stable when values have not changed', () => {
    let renders = 0;
    const { result, rerender } = renderHook(() => {
      renders += 1;
      return useStudioFeatureFlags();
    });
    const [snapshot, setter] = result.current;
    rerender();
    expect(result.current[0]).toBe(snapshot);
    expect(result.current[1]).toBe(setter);
    const beforeEvent = renders;
    act(() => {
      window.localStorage.setItem(STORAGE_KEY, '{"streamTowers":false,"laneMesh":false}');
      changedStorage();
    });
    expect(result.current[0]).toBe(snapshot);
    expect(renders).toBe(beforeEvent);
  });

  it('does not write defaults while mounting a consumer', () => {
    const setItem = vi.spyOn(Storage.prototype, 'setItem');
    renderHook(useStudioFeatureFlags);
    expect(setItem).not.toHaveBeenCalled();
  });

  it('observes changes between render and subscription without a storage event', () => {
    const { result } = renderHook(() => {
      const flags = useStudioFeatureFlags();
      useLayoutEffect(() => {
        window.localStorage.setItem(STORAGE_KEY, '{"laneMesh":true,"streamTowers":false}');
      }, []);
      return flags;
    });
    expect(result.current[0]).toEqual({ laneMesh: true, streamTowers: false, stateMachine: false });
  });

  it('removes its shared storage listener after the last consumer unmounts', () => {
    const add = vi.spyOn(window, 'addEventListener');
    const remove = vi.spyOn(window, 'removeEventListener');
    const first = renderHook(useStudioFeatureFlags);
    const second = renderHook(useStudioFeatureFlags);
    const registrations = add.mock.calls.filter(([event]) => event === 'storage');
    expect(registrations).toHaveLength(1);
    first.unmount();
    expect(remove.mock.calls.filter(([event]) => event === 'storage')).toHaveLength(0);
    second.unmount();
    expect(remove.mock.calls.filter(([event]) => event === 'storage')).toEqual([['storage', registrations[0][1]]]);
  });

  it('uses disabled defaults for the server snapshot even if this browser enabled a preview', () => {
    window.localStorage.setItem(STORAGE_KEY, '{"laneMesh":true,"streamTowers":true}');
    function Flags() {
      return JSON.stringify(useStudioFeatureFlags()[0]);
    }
    expect(renderToString(createElement(Flags))).toContain(
      '{&quot;laneMesh&quot;:false,&quot;streamTowers&quot;:false,&quot;stateMachine&quot;:false}',
    );
  });

  it('shares in-memory updates and preserves them across remounts when writes fail', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('Full', 'QuotaExceededError');
    });
    const first = renderHook(useStudioFeatureFlags);
    const second = renderHook(useStudioFeatureFlags);
    act(() => first.result.current[1]('laneMesh', true));
    expect(second.result.current[0]).toEqual({ laneMesh: true, streamTowers: false, stateMachine: false });
    first.unmount();
    second.unmount();

    const reloaded = renderHook(useStudioFeatureFlags);
    expect(reloaded.result.current[0]).toEqual({ laneMesh: true, streamTowers: false, stateMachine: false });
    act(() => reloaded.result.current[1]('streamTowers', true));
    expect(reloaded.result.current[0]).toEqual({ laneMesh: true, streamTowers: true, stateMachine: false });
  });

  it('shares in-memory updates when the localStorage getter is blocked', () => {
    vi.spyOn(window, 'localStorage', 'get').mockImplementation(() => {
      throw new DOMException('Blocked', 'SecurityError');
    });
    const first = renderHook(useStudioFeatureFlags);
    const second = renderHook(useStudioFeatureFlags);
    act(() => first.result.current[1]('laneMesh', true));
    expect(second.result.current[0]).toEqual({ laneMesh: true, streamTowers: false, stateMachine: false });
    act(() => second.result.current[1]('streamTowers', true));
    expect(first.result.current[0]).toEqual({ laneMesh: true, streamTowers: true, stateMachine: false });
  });

  it('merges pending choices with persisted flags once blocked storage becomes readable', () => {
    window.localStorage.setItem(STORAGE_KEY, '{"laneMesh":false,"streamTowers":true}');
    const getter = vi.spyOn(window, 'localStorage', 'get').mockImplementation(() => {
      throw new DOMException('Blocked', 'SecurityError');
    });
    const { result } = renderHook(useStudioFeatureFlags);
    act(() => result.current[1]('laneMesh', true));
    getter.mockRestore();

    act(() => result.current[1]('laneMesh', true));

    expect(result.current[0]).toEqual({ laneMesh: true, streamTowers: true, stateMachine: false });
    expect(JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? 'null')).toEqual({
      laneMesh: true,
      streamTowers: true,
      stateMachine: false,
    });
  });

  it('keeps toggles usable when storage cannot be read or written', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('Blocked', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('Full', 'QuotaExceededError');
    });

    const { result } = renderHook(useStudioFeatureFlags);
    expect(result.current[0]).toEqual({ laneMesh: false, streamTowers: false, stateMachine: false });
    act(() => result.current[1]('laneMesh', true));
    expect(result.current[0]).toEqual({ laneMesh: true, streamTowers: false, stateMachine: false });
  });
});

import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useStudioFeatureFlags } from './feature-flags';

const STORAGE_KEY = 'trogon-atlas.studio.feature-flags';

describe('Studio feature flags', () => {
  beforeEach(() => window.localStorage.clear());
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('starts disabled and preserves independent choices across remounts', () => {
    const { result, unmount } = renderHook(useStudioFeatureFlags);
    expect(result.current[0]).toEqual({ laneMesh: false, streamTowers: false });
    act(() => result.current[1]('laneMesh', true));
    act(() => result.current[1]('streamTowers', true));
    act(() => result.current[1]('laneMesh', false));
    unmount();

    const reloaded = renderHook(useStudioFeatureFlags);
    expect(reloaded.result.current[0]).toEqual({ laneMesh: false, streamTowers: true });
  });

  it.each(['broken', 'null', '[]', 'true', '{"laneMesh":"true","streamTowers":1}'])(
    'keeps malformed stored flags disabled: %s',
    (stored) => {
      window.localStorage.setItem(STORAGE_KEY, stored);
      const { result } = renderHook(useStudioFeatureFlags);
      expect(result.current[0]).toEqual({ laneMesh: false, streamTowers: false });
    },
  );

  it('accepts only explicitly enabled known flags', () => {
    window.localStorage.setItem(STORAGE_KEY, '{"laneMesh":true,"unknown":true}');
    const { result } = renderHook(useStudioFeatureFlags);
    expect(result.current[0]).toEqual({ laneMesh: true, streamTowers: false });
  });

  it('keeps toggles usable when storage cannot be read or written', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('Blocked', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('Full', 'QuotaExceededError');
    });

    const { result } = renderHook(useStudioFeatureFlags);
    expect(result.current[0]).toEqual({ laneMesh: false, streamTowers: false });
    act(() => result.current[1]('laneMesh', true));
    expect(result.current[0]).toEqual({ laneMesh: true, streamTowers: false });
  });
});

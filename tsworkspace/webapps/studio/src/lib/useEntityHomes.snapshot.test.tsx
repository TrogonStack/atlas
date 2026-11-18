// A snapshot holds one board, so the index read would fail; the hook must
// not ask for it.
import { cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  api: {
    eventModels: vi.fn(),
  },
}));
vi.mock('@/lib/snapshot', () => ({
  activeSnapshot: () => ({ version: 1, capturedAt: '', namespace: 'shop', slug: 'buy', responses: {} }),
}));

import { api } from '@/lib/api';
import { useEntityHomes } from './useEntityHomes';

describe('useEntityHomes in a snapshot', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('reads no event-model index and reports no error', () => {
    const { result } = renderHook(() =>
      useEntityHomes([{ kind: 'event', id: { namespace: 'billing', slug: 'invoice.issued', version: '1' } }]),
    );

    expect(api.eventModels).not.toHaveBeenCalled();
    expect(result.current.error).toBeUndefined();
    expect(result.current.homes.size).toBe(0);
  });
});

// BUG proof: loadIndex keys members with a naive
// ENTITY_KIND_FOO → foo (underscored) transform, while callers pass
// wireKind/camelCase kinds (readModel). Lookups for multi-word kinds miss.
import { cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  api: {
    eventModels: vi.fn(),
  },
}));

import { api } from '@/lib/api';
import { invalidateEntityHomes, useEntityHomes } from './useEntityHomes';

describe('useEntityHomes kind key alignment', () => {
  beforeEach(() => {
    invalidateEntityHomes();
    vi.clearAllMocks();
  });

  afterEach(() => {
    cleanup();
    invalidateEntityHomes();
    vi.restoreAllMocks();
  });

  it('resolves homes for a readModel member using camelCase kind', async () => {
    (api.eventModels as ReturnType<typeof vi.fn>).mockResolvedValue({
      entities: [
        {
          eventModel: {
            id: { namespace: 'shop', slug: 'buy', version: '1' },
            title: 'Buy flow',
            members: [
              {
                kind: 'ENTITY_KIND_READ_MODEL',
                id: { namespace: 'shop', slug: 'cart', version: '1' },
              },
            ],
          },
        },
      ],
    });

    const { result } = renderHook(() =>
      useEntityHomes([{ kind: 'readModel', id: { namespace: 'shop', slug: 'cart', version: '1' } }]),
    );

    await waitFor(() => {
      expect(result.current.homes.has('shop/cart')).toBe(true);
    });
    expect(result.current.homes.get('shop/cart')?.map((e) => e.title)).toContain('Buy flow');
  });
});

describe('BUG: useEntityHomes must load event models under the active branch', () => {
  beforeEach(() => {
    invalidateEntityHomes();
    vi.clearAllMocks();
    window.history.replaceState({}, '', '/em/shop/buy?branch=alex%2Frework');
  });

  afterEach(() => {
    cleanup();
    invalidateEntityHomes();
    window.history.replaceState({}, '', '/');
    vi.restoreAllMocks();
  });

  it('passes the URL branch through to api.eventModels', async () => {
    (api.eventModels as ReturnType<typeof vi.fn>).mockResolvedValue({ entities: [] });

    renderHook(() => useEntityHomes([]));

    await waitFor(() => {
      expect(api.eventModels).toHaveBeenCalled();
    });
    expect(api.eventModels).toHaveBeenCalledWith(expect.objectContaining({ branch: 'alex/rework' }));
  });
});

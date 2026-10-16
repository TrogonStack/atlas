import { act, renderHook } from '@testing-library/react';
import { type UrlUpdateEvent, withNuqsTestingAdapter } from 'nuqs/adapters/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BranchDiffEntry } from './api';
import {
  branchStatusLookup,
  lookupBranchStatus,
  readAsOfFromUrl,
  readBranchFromUrl,
  useAsOfParam,
  useBranchParam,
  withAsOfQuery,
  withBranchQuery,
} from './branch';

const AS_OF = '0192f000-0000-7000-8000-000000000003';

function entry(overrides: Partial<BranchDiffEntry> & { ref: BranchDiffEntry['ref']; status: string }): BranchDiffEntry {
  return {
    base: null,
    baseEtag: '',
    ours: null,
    theirs: null,
    conflictFieldPaths: [],
    ...overrides,
  };
}

describe('withBranchQuery', () => {
  it('appends ?branch= to a bare path', () => {
    expect(withBranchQuery('/em/orders/checkout', 'alex/x')).toBe('/em/orders/checkout?branch=alex%2Fx');
  });

  it('appends &branch= to a path that already has a query string', () => {
    expect(withBranchQuery('/?domain=core', 'my-branch')).toBe('/?domain=core&branch=my-branch');
  });

  it('preserves a hash fragment', () => {
    expect(withBranchQuery('/em/ns/slug#top', 'my-branch')).toBe('/em/ns/slug?branch=my-branch#top');
  });

  it('removes ?branch= when branch is undefined (baseline)', () => {
    expect(withBranchQuery('/em/ns/slug?branch=old&focus=x', undefined)).toBe('/em/ns/slug?focus=x');
  });

  it('is a no-op when there is nothing to add or remove', () => {
    expect(withBranchQuery('/', undefined)).toBe('/');
  });

  it('drops ?asOf= when it sets a branch', () => {
    expect(withBranchQuery(`/em/acme/checkout?asOf=${AS_OF}`, 'alex/x')).toBe('/em/acme/checkout?branch=alex%2Fx');
  });
});

describe('withAsOfQuery', () => {
  it('sets ?asOf= and drops ?branch=, keeping the rest of the href', () => {
    expect(withAsOfQuery('/em/acme/checkout?branch=alex%2Fx&view=sequence#top', AS_OF)).toBe(
      `/em/acme/checkout?view=sequence&asOf=${AS_OF}#top`,
    );
  });

  it('removes ?asOf= when asOf is undefined', () => {
    expect(withAsOfQuery(`/em/acme/checkout?asOf=${AS_OF}&view=sequence`, undefined)).toBe(
      '/em/acme/checkout?view=sequence',
    );
  });
});

describe('readBranchFromUrl / readAsOfFromUrl', () => {
  afterEach(() => window.history.replaceState({}, '', '/'));

  it('reads a URL that carries both as the as-of view', () => {
    window.history.replaceState({}, '', `/em/acme/checkout?branch=alex/x&asOf=${AS_OF}`);
    expect(readAsOfFromUrl()).toBe(AS_OF);
    expect(readBranchFromUrl()).toBeUndefined();
  });

  it('ignores an asOf that is not a lowercase hyphenated UUID', () => {
    window.history.replaceState({}, '', '/em/acme/checkout?branch=alex/x&asOf=0192F000-0000-7000-8000-000000000003');
    expect(readAsOfFromUrl()).toBeUndefined();
    expect(readBranchFromUrl()).toBe('alex/x');
  });
});

describe('useAsOfParam / useBranchParam', () => {
  it('setting asOf clears the branch', async () => {
    const onUrlUpdate = vi.fn<(e: UrlUpdateEvent) => void>();
    const { result } = renderHook(() => useAsOfParam(), {
      wrapper: withNuqsTestingAdapter({ searchParams: '?branch=alex/x&view=sequence', onUrlUpdate }),
    });
    expect(result.current[0]).toBeUndefined();
    await act(async () => result.current[1](AS_OF));
    const params = onUrlUpdate.mock.lastCall?.[0].searchParams ?? new URLSearchParams();
    expect(params.get('asOf')).toBe(AS_OF);
    expect(params.has('branch')).toBe(false);
    expect(params.get('view')).toBe('sequence');
  });

  it('setting a branch clears asOf, and a URL with both reads no branch', async () => {
    const onUrlUpdate = vi.fn<(e: UrlUpdateEvent) => void>();
    const { result } = renderHook(() => useBranchParam(), {
      wrapper: withNuqsTestingAdapter({ searchParams: `?branch=alex/x&asOf=${AS_OF}`, onUrlUpdate }),
    });
    expect(result.current[0]).toBeUndefined();
    await act(async () => result.current[1]('alex/y'));
    const params = onUrlUpdate.mock.lastCall?.[0].searchParams ?? new URLSearchParams();
    expect(params.get('branch')).toBe('alex/y');
    expect(params.has('asOf')).toBe(false);
  });

  it('exiting the as-of view leaves the branch alone', async () => {
    const onUrlUpdate = vi.fn<(e: UrlUpdateEvent) => void>();
    const { result } = renderHook(() => useAsOfParam(), {
      wrapper: withNuqsTestingAdapter({ searchParams: `?asOf=${AS_OF}`, onUrlUpdate }),
    });
    expect(result.current[0]).toBe(AS_OF);
    await act(async () => result.current[1](undefined));
    expect(onUrlUpdate.mock.lastCall?.[0].queryString).toBe('');
  });
});

describe('branchStatusLookup', () => {
  it('maps STATUS_ADDED / STATUS_CHANGED / STATUS_DELETED to their badge names', () => {
    const entries: BranchDiffEntry[] = [
      entry({
        ref: { kind: 'ENTITY_KIND_EVENT', id: { namespace: 'ns', slug: 'added-event', version: '1' } },
        status: 'STATUS_ADDED',
      }),
      entry({
        ref: { kind: 'ENTITY_KIND_COMMAND', id: { namespace: 'ns', slug: 'changed-cmd', version: '1' } },
        status: 'STATUS_CHANGED',
      }),
      entry({
        ref: { kind: 'ENTITY_KIND_READ_MODEL', id: { namespace: 'ns', slug: 'deleted-rm', version: '1' } },
        status: 'STATUS_DELETED',
      }),
    ];
    const lookup = branchStatusLookup(entries);
    expect(lookupBranchStatus(lookup, 'event', { namespace: 'ns', slug: 'added-event' })).toBe('added');
    expect(lookupBranchStatus(lookup, 'command', { namespace: 'ns', slug: 'changed-cmd' })).toBe('changed');
    expect(lookupBranchStatus(lookup, 'readModel', { namespace: 'ns', slug: 'deleted-rm' })).toBe('deleted');
  });

  it('collapses all three STATUS_CONFLICT_* variants to "conflict"', () => {
    const variants = ['STATUS_CONFLICT_EDIT_EDIT', 'STATUS_CONFLICT_EDIT_DELETE', 'STATUS_CONFLICT_DELETE_EDIT'];
    for (const status of variants) {
      const lookup = branchStatusLookup([
        entry({ ref: { kind: 'ENTITY_KIND_EVENT', id: { namespace: 'ns', slug: 'e', version: '1' } }, status }),
      ]);
      expect(lookupBranchStatus(lookup, 'event', { namespace: 'ns', slug: 'e' })).toBe('conflict');
    }
  });

  it('ignores STATUS_CONVERGED and STATUS_UNSPECIFIED (nothing to render)', () => {
    const lookup = branchStatusLookup([
      entry({ ref: { kind: 'ENTITY_KIND_EVENT', id: { namespace: 'ns', slug: 'a' } }, status: 'STATUS_CONVERGED' }),
      entry({ ref: { kind: 'ENTITY_KIND_EVENT', id: { namespace: 'ns', slug: 'b' } }, status: 'STATUS_UNSPECIFIED' }),
    ]);
    expect(lookup.size).toBe(0);
  });

  it('ignores entries with an unresolvable kind or a missing ref/id', () => {
    const lookup = branchStatusLookup([
      entry({ ref: { kind: 'ENTITY_KIND_UNKNOWN', id: { namespace: 'ns', slug: 'a' } }, status: 'STATUS_ADDED' }),
      entry({ ref: null, status: 'STATUS_ADDED' }),
      entry({ ref: { kind: 'ENTITY_KIND_EVENT', id: undefined }, status: 'STATUS_ADDED' }),
    ]);
    expect(lookup.size).toBe(0);
  });

  it('matches version-agnostically: a diff entry pins no particular rendered occurrence version', () => {
    const lookup = branchStatusLookup([
      entry({
        ref: { kind: 'ENTITY_KIND_EVENT', id: { namespace: 'ns', slug: 'e', version: '3' } },
        status: 'STATUS_CHANGED',
      }),
    ]);
    // A sticky rendering v1 of the same (kind, namespace, slug) still matches.
    expect(lookupBranchStatus(lookup, 'event', { namespace: 'ns', slug: 'e' })).toBe('changed');
  });
});

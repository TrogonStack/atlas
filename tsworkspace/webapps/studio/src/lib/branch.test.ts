import { describe, expect, it } from 'vitest';
import type { BranchDiffEntry } from './api';
import { branchStatusLookup, lookupBranchStatus, withBranchQuery } from './branch';

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

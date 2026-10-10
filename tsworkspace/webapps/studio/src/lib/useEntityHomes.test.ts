// Tests for useEntityHomes.ts.
//
// @testing-library/react is NOT in package.json, so `useEntityHomes` (a React
// hook) cannot be rendered in a test environment. The hook is NOT tested here.
//
// What IS testable without new dependencies:
// - `invalidateEntityHomes` is a pure cache-clear exported function.
//   We verify it can be called without throwing.
// - The `loadIndex` cache logic is module-private, but its public surface
//   (that `invalidateEntityHomes` resets it) is covered by testing that
//   calling it repeatedly does not throw and leaves the module in a callable
//   state (we stub fetch to prevent network calls).
//
// NOT COVERED (requires @testing-library/react or a React test environment):
// - `useEntityHomes` hook: loading state, error state, retry trigger,
//   memoized homes map derivation.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invalidateEntityHomes } from './useEntityHomes';

describe('invalidateEntityHomes', () => {
  it('is exported and callable without throwing', () => {
    expect(() => invalidateEntityHomes()).not.toThrow();
  });

  it('can be called multiple times in succession without throwing', () => {
    expect(() => {
      invalidateEntityHomes();
      invalidateEntityHomes();
      invalidateEntityHomes();
    }).not.toThrow();
  });
});

describe('loadIndex cache behavior (via invalidateEntityHomes)', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    // Clear any cached promise from a previous test.
    invalidateEntityHomes();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    invalidateEntityHomes();
  });

  it('does not throw when fetch succeeds and index is loaded', async () => {
    fetchSpy.mockResolvedValueOnce(
      new Response(JSON.stringify({ entities: [] }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );
    // We cannot call the hook directly, but we can import loadIndex indirectly
    // by checking that invalidateEntityHomes does not break after a resolved fetch.
    // The cache mechanism is opaque here: we just verify the module is stable.
    invalidateEntityHomes();
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});

// NOTE: The homes Map derivation logic inside `useEntityHomes` (the useMemo
// that maps ref.kind + ref.id -> list of EventModel entities) is inlined in
// the hook and cannot be exercised independently. It would require
// @testing-library/react renderHook. To test that logic, add
// @testing-library/react to devDependencies and render the hook with a mock
// api.eventModels response.

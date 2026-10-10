import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  authHeaders,
  clearCredential,
  getCredential,
  onUnauthenticated,
  resetCredentialCacheForTests,
  setCredential,
  subscribeCredential,
} from './credential';

function makeStorageShim() {
  const map = new Map<string, string>();
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => void map.set(k, v),
    removeItem: (k: string) => void map.delete(k),
    _map: map,
  };
}

let storageShim: ReturnType<typeof makeStorageShim>;

beforeEach(() => {
  storageShim = makeStorageShim();
  vi.stubGlobal('window', { sessionStorage: storageShim });
  resetCredentialCacheForTests();
});

afterEach(() => {
  vi.unstubAllGlobals();
  resetCredentialCacheForTests();
});

describe('credential storage', () => {
  it('has no credential until one is set', () => {
    expect(getCredential()).toBeNull();
    expect(authHeaders()).toEqual({});
  });

  it('sends the credential as a bearer token once set', () => {
    setCredential('alice-key');
    expect(authHeaders()).toEqual({ Authorization: 'Bearer alice-key' });
  });

  it('trims surrounding whitespace, which pasting a key routinely adds', () => {
    setCredential('  alice-key\n');
    expect(getCredential()).toBe('alice-key');
  });

  it('treats a whitespace-only value as no credential rather than a valid one', () => {
    setCredential('   ');
    expect(getCredential()).toBeNull();
    expect(authHeaders()).toEqual({});
  });

  it('survives a reload within the tab', () => {
    setCredential('alice-key');
    resetCredentialCacheForTests();
    expect(getCredential()).toBe('alice-key');
  });

  it('leaves nothing behind when cleared', () => {
    setCredential('alice-key');
    clearCredential();
    expect(getCredential()).toBeNull();
    expect([...storageShim._map.values()]).not.toContain('alice-key');
  });

  it('drops a credential the server rejected, so the next request can prompt', () => {
    setCredential('stale-key');
    onUnauthenticated();
    expect(getCredential()).toBeNull();
  });

  it('notifies subscribers when the credential changes', () => {
    const seen: (string | null)[] = [];
    const unsubscribe = subscribeCredential(() => seen.push(getCredential()));
    setCredential('alice-key');
    clearCredential();
    unsubscribe();
    setCredential('ignored-after-unsubscribe');
    expect(seen).toEqual(['alice-key', null]);
  });

  it('falls back to memory when sessionStorage is unavailable', () => {
    vi.stubGlobal('window', {
      get sessionStorage(): Storage {
        throw new Error('blocked by cookie policy');
      },
    });
    resetCredentialCacheForTests();
    setCredential('alice-key');
    expect(getCredential()).toBe('alice-key');
  });
});

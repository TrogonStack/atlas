// Tests for api.ts: URL construction and fetch handling.
// Uses vi.stubGlobal to stub fetch and window (api.ts uses window.setTimeout/clearTimeout).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from './api';

// api.ts calls window.setTimeout / window.clearTimeout. In a Node test
// environment `window` is undefined. We install a minimal shim before each
// test and restore it after.
function makeWindowShim() {
  const timeoutIds = new Set<ReturnType<typeof globalThis.setTimeout>>();
  return {
    setTimeout: (fn: () => void, ms: number) => {
      const id = globalThis.setTimeout(fn, ms);
      timeoutIds.add(id);
      return id;
    },
    clearTimeout: (id: ReturnType<typeof globalThis.setTimeout>) => {
      timeoutIds.delete(id);
      globalThis.clearTimeout(id);
    },
  };
}

const makeOkResponse = (body: unknown) =>
  new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });

const makeErrorResponse = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), {
    status,
    statusText: 'Error',
    headers: { 'Content-Type': 'application/json' },
  });

describe('api.info', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('fetches /api/info and returns parsed JSON on success', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ schemaVersion: '1', serverVersion: '2' }));
    const result = await api.info();
    expect(fetchSpy).toHaveBeenCalledOnce();
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/info');
    expect(result.schemaVersion).toBe('1');
    expect(result.serverVersion).toBe('2');
  });

  it('throws with the server error message when the response is not ok', async () => {
    fetchSpy.mockResolvedValueOnce(makeErrorResponse(404, { error: 'not found' }));
    await expect(api.info()).rejects.toThrow('not found');
  });

  it('throws with status/statusText when the error body has no error field', async () => {
    fetchSpy.mockResolvedValueOnce(makeErrorResponse(500, {}));
    await expect(api.info()).rejects.toThrow('500');
  });
});

describe('api.namespaces', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('fetches /api/namespaces', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ namespaces: ['a', 'b'] }));
    const result = await api.namespaces();
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/namespaces');
    expect(result.namespaces).toEqual(['a', 'b']);
  });
});

describe('api namespace registry', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('fetches the registry from its own path, not the picker', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ namespaces: [] }));
    await api.namespaceRegistry();
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/namespaces/registry');
  });

  it('offers no way to mutate the registry, since agents own every edit', () => {
    expect(api).not.toHaveProperty('registerNamespace');
    expect(api).not.toHaveProperty('moveNamespace');
  });
});

describe('api.model URL construction', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('uses /api/model with no query when namespaces is empty', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ entities: [] }));
    await api.model([]);
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/model');
  });

  it('uses /api/model with no query when namespaces is undefined', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ entities: [] }));
    await api.model(undefined);
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/model');
  });

  it('appends comma-joined namespace param when namespaces are provided', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ entities: [] }));
    await api.model(['orders', 'payments']);
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/model?namespace=orders%2Cpayments');
  });
});

describe('api.search URL construction', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('encodes query string and omits namespace when not provided', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ results: [] }));
    await api.search('hello world');
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/search?q=hello%20world');
  });

  it('appends namespace param when provided', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ results: [] }));
    await api.search('event', ['orders']);
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toContain('q=event');
    expect(url).toContain('namespace=orders');
  });

  it('carries the branch, which it used to drop on the floor', async () => {
    // Dropping it made a search during a branch preview answer from
    // baseline: the entity the branch had just added was not findable.
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ results: [] }));
    await api.search('event', undefined, { branch: 'alex/retention-rework' });
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toContain('branch=alex%2Fretention-rework');
  });
});

describe('api.changes URL construction', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('encodes the after token in the URL', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ events: [] }));
    await api.changes('tok/special=value');
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toContain('/api/changes?after=');
    expect(decodeURIComponent(url.split('after=')[1])).toBe('tok/special=value');
  });
});

describe('api branch threading', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('does not append ?branch= when opts.branch is unset (baseline, byte-identical)', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ schemaVersion: '1' }));
    await api.info();
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/info');
  });

  it('appends ?branch= to a bare URL', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ namespaces: [] }));
    await api.namespaces({ branch: 'alex/retention-rework' });
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/namespaces?branch=alex%2Fretention-rework');
  });

  it('appends &branch= to a URL that already has a query string', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ entities: [] }));
    await api.model(['orders'], { branch: 'my-branch' });
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/model?namespace=orders&branch=my-branch');
  });

  it('threads branch through api.changes', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ events: [] }));
    await api.changes('tok', { branch: 'my-branch' });
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/changes?after=tok&branch=my-branch');
  });

  it('threads branch through api.overview', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ entities: [] }));
    await api.overview({ branch: 'my-branch' });
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/overview?branch=my-branch');
  });

  it('threads branch through api.eventModels', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ entities: [] }));
    await api.eventModels({ branch: 'my-branch' });
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/event-models?branch=my-branch');
  });

  it('threads branch through api.eventModel', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ entities: [] }));
    await api.eventModel('ns', 'slug', { branch: 'my-branch' });
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/event-model/ns/slug?branch=my-branch');
  });
});

describe('api.branches / api.branchDiff', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('fetches /api/branches', async () => {
    fetchSpy.mockResolvedValueOnce(
      makeOkResponse({ branches: [{ name: 'alex/x', doc: '', createdAt: '', deltaCount: 1 }] }),
    );
    const result = await api.branches();
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/branches');
    expect(result.branches).toHaveLength(1);
  });

  it('encodes the name query param on /api/branch-diff', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ entries: [] }));
    await api.branchDiff('alex/retention-rework');
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/branch-diff?name=alex%2Fretention-rework');
  });
});

describe('api.eventModel URL construction', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('encodes namespace and slug in the path', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ entities: [] }));
    await api.eventModel('my ns', 'my slug');
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/event-model/my%20ns/my%20slug');
  });
});

describe('BUG: timeout and error parsing', () => {
  beforeEach(() => {
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('rejects with Error("request timed out") instead of AbortError so callers do not swallow timeouts', async () => {
    vi.stubGlobal('fetch', (_url: string, init?: RequestInit) => {
      return new Promise((_resolve, reject) => {
        const signal = init?.signal;
        if (!signal) return;
        const onAbort = () => reject(new DOMException('The operation was aborted.', 'AbortError'));
        if (signal.aborted) onAbort();
        else signal.addEventListener('abort', onAbort, { once: true });
      });
    });

    try {
      await api.info({ timeoutMs: 20 });
      expect.unreachable('should reject');
    } catch (e) {
      const err = e as Error;
      expect(err.name).not.toBe('AbortError');
      expect(err.message).toMatch(/timed out/i);
    }
  });

  it('surfaces body.message when the error envelope uses message instead of error', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValueOnce(
        new Response(JSON.stringify({ message: 'upstream refused' }), {
          status: 503,
          statusText: 'Service Unavailable',
          headers: { 'Content-Type': 'application/json' },
        }),
      ),
    );
    await expect(api.info()).rejects.toThrow('upstream refused');
  });

  it('surfaces plain-text error bodies when JSON parsing fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValueOnce(
        new Response('gateway exploded', {
          status: 502,
          statusText: 'Bad Gateway',
          headers: { 'Content-Type': 'text/plain' },
        }),
      ),
    );
    await expect(api.info()).rejects.toThrow('gateway exploded');
  });
});

describe('BUG: validate / validate-project client surface', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('api.validate encodes namespace, slug, and version query params', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ issues: [] }));
    await api.validate({ namespace: 'shop', slug: 'orders', version: '2' });
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toContain('/api/validate?');
    expect(url).toContain('namespace=shop');
    expect(url).toContain('slug=orders');
    expect(url).toContain('version=2');
  });

  it('api.validateProject encodes projectVersion (and domainVersion) query params', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ reports: [], totalErrors: 0 }));
    await api.validateProject({
      projectNamespace: 'acme',
      projectSlug: 'platform',
      projectVersion: '3',
    });
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toContain('/api/validate-project?');
    expect(url).toContain('projectNamespace=acme');
    expect(url).toContain('projectSlug=platform');
    expect(url).toContain('projectVersion=3');
  });

  it('api.validateProject threads ?branch= for branch-scoped validation', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ reports: [] }));
    await api.validateProject(
      { domainNamespace: 'acme', domainSlug: 'commerce', domainVersion: '1' },
      { branch: 'alex/rework' },
    );
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toContain('domainNamespace=acme');
    expect(url).toContain('domainVersion=1');
    expect(url).toContain('branch=alex%2Frework');
  });

  it('offers no bulk deletion, since agents own every edit', () => {
    expect(api).not.toHaveProperty('deleteByQuery');
  });
});

describe('BUG: changes pagination query params', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('api.changes accepts pageSize and forwards it as pageSize query param', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ events: [], nextToken: 'next' }));
    await api.changes('tok', { pageSize: 25 });
    const [url] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(url).toContain('/api/changes?');
    expect(url).toContain('after=tok');
    expect(url).toContain('pageSize=25');
  });
});

describe('BUG: branch forwarded as x-trogon-atlas-branch request header', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', makeWindowShim());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('sets x-trogon-atlas-branch header when opts.branch is set (in addition to ?branch=)', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ entities: [] }));
    await api.model(['orders'], { branch: 'alex/retention-rework' });
    const [, init] = fetchSpy.mock.calls[0] as [string, RequestInit];
    const headers = new Headers(init.headers);
    expect(headers.get('x-trogon-atlas-branch')).toBe('alex/retention-rework');
  });

  it('omits x-trogon-atlas-branch header on baseline (no branch)', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ schemaVersion: '1' }));
    await api.info();
    const [, init] = fetchSpy.mock.calls[0] as [string, RequestInit];
    const headers = new Headers(init?.headers);
    expect(headers.get('x-trogon-atlas-branch')).toBeNull();
  });
});

describe('api credential forwarding', () => {
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    fetchSpy = vi.fn();
    const storage = new Map<string, string>();
    vi.stubGlobal('fetch', fetchSpy);
    vi.stubGlobal('window', {
      ...makeWindowShim(),
      sessionStorage: {
        getItem: (k: string) => storage.get(k) ?? null,
        setItem: (k: string, v: string) => void storage.set(k, v),
        removeItem: (k: string) => void storage.delete(k),
      },
    });
    const credential = await import('./credential');
    credential.resetCredentialCacheForTests();
  });

  afterEach(async () => {
    const credential = await import('./credential');
    credential.resetCredentialCacheForTests();
    vi.unstubAllGlobals();
  });

  it('sends no Authorization header when the browser has no credential', async () => {
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ schemaVersion: '1', serverVersion: '2' }));
    await api.info();
    const [, init] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(init.headers).not.toHaveProperty('Authorization');
  });

  it('attaches the browser credential to reads', async () => {
    const { setCredential } = await import('./credential');
    setCredential('alice-key');
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ schemaVersion: '1', serverVersion: '2' }));
    await api.info();
    const [, init] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(init.headers).toMatchObject({ Authorization: 'Bearer alice-key' });
  });

  it('keeps the branch header alongside the credential rather than replacing it', async () => {
    const { setCredential } = await import('./credential');
    setCredential('alice-key');
    fetchSpy.mockResolvedValueOnce(makeOkResponse({ namespaces: [], labels: {} }));
    await api.namespaces({ branch: 'alex/retention-rework' });
    const [, init] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(init.headers).toMatchObject({
      Authorization: 'Bearer alice-key',
      'x-trogon-atlas-branch': 'alex/retention-rework',
    });
  });

  it('forgets a credential the bridge rejected and tells subscribers once', async () => {
    const { setCredential, getCredential } = await import('./credential');
    const { subscribeUnauthenticated } = await import('./api');
    setCredential('stale-key');
    const seen: string[] = [];
    const unsubscribe = subscribeUnauthenticated((message) => seen.push(message));
    fetchSpy.mockResolvedValueOnce(makeErrorResponse(401, { error: 'unauthenticated' }));
    await expect(api.info()).rejects.toThrow('unauthenticated');
    unsubscribe();
    expect(seen).toEqual(['unauthenticated']);
    // A rejected key left in storage would make every later request fail the
    // same way with nothing on screen to explain it.
    expect(getCredential()).toBeNull();
  });

  it('does not treat a 403 as a bad credential', async () => {
    const { setCredential, getCredential } = await import('./credential');
    setCredential('reader-key');
    fetchSpy.mockResolvedValueOnce(makeErrorResponse(403, { error: 'permission denied' }));
    await expect(api.info()).rejects.toThrow('permission denied');
    // The key is real; it just does not reach this namespace. Dropping it
    // would log the reader out for asking one question too many.
    expect(getCredential()).toBe('reader-key');
  });
});

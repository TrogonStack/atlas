import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { parseFrames, watchChangesViaSse } from './change-stream';
import { resetCredentialCacheForTests, setCredential } from './credential';

const { reportUnauthenticated } = vi.hoisted(() => ({ reportUnauthenticated: vi.fn() }));
vi.mock('@/lib/api', () => ({ reportUnauthenticated }));

describe('parseFrames', () => {
  it('returns nothing while a frame is still incomplete', () => {
    const { frames, rest } = parseFrames('id: 7\ndata: {"events":[]}');
    expect(frames).toEqual([]);
    expect(rest).toBe('id: 7\ndata: {"events":[]}');
  });

  it('parses a complete frame and keeps the partial tail', () => {
    const { frames, rest } = parseFrames('id: 7\ndata: {"a":1}\n\nid: 8\ndata: {"b"');
    expect(frames).toEqual([{ event: 'message', data: '{"a":1}', id: '7' }]);
    expect(rest).toBe('id: 8\ndata: {"b"');
  });

  it('skips heartbeat comments', () => {
    const { frames } = parseFrames(': keep-alive\n\n');
    expect(frames).toEqual([]);
  });

  it('reads the event name', () => {
    const { frames } = parseFrames('event: fatal\ndata: {"message":"gone"}\n\n');
    expect(frames[0]?.event).toBe('fatal');
  });
});

/**
 * A response body yielding the given chunks. With a signal it stays open until
 * aborted, standing in for a long-lived stream; without one it ends after the
 * chunks, standing in for a server that closed the connection.
 */
function streamOf(chunks: string[], signal?: AbortSignal): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
      if (!signal) {
        controller.close();
        return;
      }
      signal.addEventListener('abort', () => {
        try {
          controller.close();
        } catch {
          // Already closed.
        }
      });
    },
  });
}

describe('watchChangesViaSse', () => {
  beforeEach(() => {
    resetCredentialCacheForTests();
    reportUnauthenticated.mockClear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    resetCredentialCacheForTests();
  });

  it('carries the caller credential and reports live', async () => {
    setCredential('key-abc');
    let seen: Record<string, string> = {};
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_url: string, init: RequestInit) => {
        seen = init.headers as Record<string, string>;
        return { ok: true, status: 200, body: streamOf([], init.signal ?? undefined) };
      }),
    );
    const states: string[] = [];
    const handle = watchChangesViaSse(vi.fn(), (s) => states.push(s));
    await vi.waitFor(() => expect(states).toContain('live'));
    expect(seen.Authorization).toBe('Bearer key-abc');
    expect(seen.Accept).toBe('text/event-stream');
    handle.close();
  });

  it('emits one change per event in a frame', async () => {
    const frame =
      'id: tok-9\ndata: ' +
      JSON.stringify({
        events: [
          { kind: 'KIND_PUT', token: 'e1', entity: { id: { namespace: 'ns', slug: 'order.placed', version: '1' } } },
          { kind: 'KIND_DELETED', token: 'e2', entity: { id: { namespace: 'ns', slug: 'gone', version: '1' } } },
        ],
      }) +
      '\n\n';
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_url: string, init: RequestInit) => ({
        ok: true,
        status: 200,
        body: streamOf([frame], init.signal ?? undefined),
      })),
    );
    const changes: Array<{ key: string; operation: string; revision: string }> = [];
    const handle = watchChangesViaSse((c) => changes.push(c), vi.fn());
    await vi.waitFor(() => expect(changes).toHaveLength(2));
    expect(changes[0]).toEqual({ key: 'ns.order.placed.1', operation: 'PUT', revision: 'e1' });
    expect(changes[1]?.operation).toBe('DEL');
    handle.close();
  });

  it('resumes from the last id it saw', async () => {
    const calls: Array<Record<string, string>> = [];
    let call = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_url: string, init: RequestInit) => {
        calls.push(init.headers as Record<string, string>);
        call += 1;
        if (call === 1) {
          return {
            ok: true,
            status: 200,
            body: streamOf(['id: tok-42\ndata: {"events":[]}\n\n']),
          };
        }
        return { ok: true, status: 200, body: streamOf([], init.signal ?? undefined) };
      }),
    );
    const handle = watchChangesViaSse(vi.fn(), vi.fn());
    await vi.waitFor(() => expect(calls.length).toBeGreaterThan(1), { timeout: 3000 });
    expect(calls[0]?.['Last-Event-ID']).toBeUndefined();
    expect(calls[1]?.['Last-Event-ID']).toBe('tok-42');
    handle.close();
  });

  it('reports offline and retries when the stream is refused', async () => {
    const fetchMock = vi.fn(async () => ({ ok: false, status: 401, statusText: 'Unauthorized', body: null }));
    vi.stubGlobal('fetch', fetchMock);
    const states: Array<[string, string | undefined]> = [];
    const handle = watchChangesViaSse(vi.fn(), (s, d) => states.push([s, d]));
    await vi.waitFor(() => expect(states.some(([s]) => s === 'offline')).toBe(true));
    expect(states.find(([s]) => s === 'offline')?.[1]).toContain('401');
    await vi.waitFor(() => expect(fetchMock.mock.calls.length).toBeGreaterThan(1), { timeout: 3000 });
    handle.close();
  });

  it('surfaces a fatal frame as offline instead of waiting on a dead stream', async () => {
    // The bridge sends `fatal` when it has given up on upstream and is about
    // to end the stream. Reading it as ordinary data would leave the client
    // reporting `live` against a socket that is never going to speak again.
    const frame = 'event: fatal\ndata: {"message":"upstream unavailable, closing stream"}\n\n';
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_url: string, init: RequestInit) => ({
        ok: true,
        status: 200,
        body: streamOf([frame], init.signal ?? undefined),
      })),
    );
    const states: Array<[string, string | undefined]> = [];
    const handle = watchChangesViaSse(vi.fn(), (s, d) => states.push([s, d]));
    await vi.waitFor(() => expect(states.some(([s]) => s === 'offline')).toBe(true), {
      timeout: 3000,
    });
    expect(states.find(([s]) => s === 'offline')?.[1]).toContain('upstream unavailable');
    handle.close();
  });

  it('raises the credential prompt when the stream is refused for the key', async () => {
    // The bridge vouches for the credential before it opens the stream, so a
    // 401 here means the key, and nothing else in this module runs through
    // the fetch wrapper that would otherwise surface it.
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({ ok: false, status: 401, statusText: 'Unauthorized', body: null })),
    );
    const handle = watchChangesViaSse(vi.fn(), vi.fn());
    await vi.waitFor(() => expect(reportUnauthenticated).toHaveBeenCalled());
    expect(reportUnauthenticated.mock.calls[0]?.[0]).toContain('401');
    handle.close();
  });

  it('leaves the prompt alone when the stream is merely unavailable', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({ ok: false, status: 503, statusText: 'Service Unavailable', body: null })),
    );
    const states: string[] = [];
    const handle = watchChangesViaSse(vi.fn(), (s) => states.push(s));
    await vi.waitFor(() => expect(states).toContain('offline'));
    expect(reportUnauthenticated).not.toHaveBeenCalled();
    handle.close();
  });

  it('reconnects on an error frame rather than reporting a stream that stopped delivering', async () => {
    // The bridge writes nothing between changes, so a stream that started
    // failing upstream is indistinguishable from a quiet one. Ending the
    // connection is what lets the next attempt learn which it is.
    let call = 0;
    const fetchMock = vi.fn(async (_url: string, init: RequestInit) => {
      call += 1;
      if (call === 1) {
        return {
          ok: true,
          status: 200,
          body: streamOf(['event: error\ndata: {"message":"upstream refused"}\n\n'], init.signal ?? undefined),
        };
      }
      return { ok: true, status: 200, body: streamOf([], init.signal ?? undefined) };
    });
    vi.stubGlobal('fetch', fetchMock);
    const states: Array<[string, string | undefined]> = [];
    const handle = watchChangesViaSse(vi.fn(), (s, d) => states.push([s, d]));
    await vi.waitFor(() => expect(states.some(([s]) => s === 'offline')).toBe(true), { timeout: 3000 });
    expect(states.find(([s]) => s === 'offline')?.[1]).toContain('upstream refused');
    await vi.waitFor(() => expect(fetchMock.mock.calls.length).toBeGreaterThan(1), { timeout: 3000 });
    handle.close();
  });

  it('stops fetching once closed', async () => {
    const fetchMock = vi.fn(async () => ({ ok: false, status: 500, statusText: 'boom', body: null }));
    vi.stubGlobal('fetch', fetchMock);
    const handle = watchChangesViaSse(vi.fn(), vi.fn());
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalled());
    handle.close();
    const after = fetchMock.mock.calls.length;
    await new Promise((r) => setTimeout(r, 250));
    expect(fetchMock.mock.calls.length).toBeLessThanOrEqual(after + 1);
  });
});

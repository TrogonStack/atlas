import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { KVChange, RealtimeWatch } from './realtime';

// ---------------------------------------------------------------------------
// nats.ws mock
//
// watchEntitiesUnshared drives three async loops against a NatsConnection:
//   1. the status() iterator (for reconnect events)
//   2. the KV watch iterator (for entity change events)
//   3. the dial/retry loop
//
// The mock exposes control handles so each test can push entries into the
// watch iterator and verify the callbacks it triggers.
// ---------------------------------------------------------------------------

type KvEntry = { key: string; operation: string; revision: number | bigint; length?: number };

function makeNatsConnMock(entries: KvEntry[] = []) {
  // Yields the provided entries then returns (loop exits cleanly).
  async function* watchGen() {
    for (const e of entries) yield e;
  }

  // status() returns an iterable that returns immediately (no reconnect events).
  async function* statusGen() {}

  const closeMock = vi.fn().mockResolvedValue(undefined);

  const conn = {
    close: closeMock,
    status: () => statusGen(),
    jetstream: () => ({
      views: {
        kv: async () => ({
          watch: async () => watchGen(),
        }),
      },
    }),
  };
  return { conn, closeMock };
}

vi.mock('nats.ws', () => ({
  connect: vi.fn(),
}));

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function stubFetch(token: string | null = null, transport: string = 'nats') {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ token, transport }),
    } as Response),
  );
}

// BroadcastChannel is available in jsdom. We replace it with a minimal stub
// that never delivers messages to peer tabs, so watchEntities always elects
// itself as leader and falls through to watchEntitiesUnshared.
class NoopBroadcastChannel {
  onmessage: ((ev: MessageEvent) => void) | null = null;
  postMessage(_data: unknown) {}
  close() {}
}

class PeerBroadcastChannel {
  static peers = new Set<PeerBroadcastChannel>();
  onmessage: ((ev: MessageEvent) => void) | null = null;
  closed = false;

  constructor(readonly name: string) {
    PeerBroadcastChannel.peers.add(this);
  }

  postMessage(data: unknown) {
    if (this.closed) throw new DOMException('Channel is closed', 'InvalidStateError');
    for (const peer of PeerBroadcastChannel.peers) {
      if (peer === this || peer.name !== this.name) continue;
      queueMicrotask(() => {
        if (!peer.closed) peer.onmessage?.({ data } as MessageEvent);
      });
    }
  }

  close() {
    this.closed = true;
    PeerBroadcastChannel.peers.delete(this);
  }
}

describe('watchEntities coordination', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
    vi.stubGlobal('BroadcastChannel', PeerBroadcastChannel);
    vi.stubGlobal('crypto', { randomUUID: vi.fn().mockReturnValueOnce('a').mockReturnValueOnce('b') });
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    for (const peer of PeerBroadcastChannel.peers) peer.close();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.doUnmock('@/lib/change-stream');
    vi.resetModules();
    vi.restoreAllMocks();
    vi.resetAllMocks();
  });

  it.each([
    ['live', undefined],
    ['offline', 'reconnecting'],
  ] as const)('replays the established leader status %s after a late watcher yields', async (state, detail) => {
    const closeLeader = vi.fn();
    const closeFollower = vi.fn();
    const viaSse = vi
      .fn()
      .mockImplementationOnce((_onChange, onState) => {
        onState(state, detail);
        return { close: closeLeader };
      })
      .mockReturnValueOnce({ close: closeFollower });
    vi.doMock('@/lib/change-stream', () => ({ watchChangesViaSse: viaSse }));
    stubFetch(null, 'sse');
    const { watchEntities } = await import('@/lib/realtime');
    const leader = await watchEntities(vi.fn(), vi.fn());
    await vi.advanceTimersByTimeAsync(0);
    const followerState = vi.fn();
    const follower = await watchEntities(vi.fn(), followerState);
    try {
      await vi.advanceTimersByTimeAsync(800);
      expect(closeFollower).toHaveBeenCalledOnce();
      expect(closeLeader).not.toHaveBeenCalled();
      expect(followerState).toHaveBeenLastCalledWith(state, detail);
    } finally {
      follower.close();
      leader.close();
    }
  });

  it('closes a connection that finishes dialing after its watcher closes', async () => {
    const { conn, closeMock } = makeNatsConnMock();
    let resolveConnection: ((connection: typeof conn) => void) | undefined;
    const { connect } = await import('nats.ws');
    (connect as ReturnType<typeof vi.fn>).mockReturnValue(
      new Promise((resolve) => {
        resolveConnection = resolve;
      }),
    );
    stubFetch('test-token');
    const { watchEntities } = await import('@/lib/realtime');
    const onState = vi.fn();
    const handle = await watchEntities(vi.fn(), onState);
    await vi.advanceTimersByTimeAsync(0);
    handle.close();
    onState.mockClear();
    resolveConnection?.(conn);
    await vi.advanceTimersByTimeAsync(0);
    expect(closeMock).toHaveBeenCalledOnce();
    expect(onState).not.toHaveBeenCalled();
  });

  it('ignores late state callbacks from a connection that yielded leadership', async () => {
    let staleState: ((state: 'connecting' | 'live' | 'offline', detail?: string) => void) | undefined;
    const closeFollower = vi.fn();
    const viaSse = vi
      .fn()
      .mockImplementationOnce((_onChange, onState) => {
        onState('live');
        return { close: vi.fn() };
      })
      .mockImplementationOnce((_onChange, onState) => {
        staleState = onState;
        return { close: closeFollower };
      });
    vi.doMock('@/lib/change-stream', () => ({ watchChangesViaSse: viaSse }));
    stubFetch(null, 'sse');
    const { watchEntities } = await import('@/lib/realtime');
    const leader = await watchEntities(vi.fn(), vi.fn());
    await vi.advanceTimersByTimeAsync(0);
    const onState = vi.fn();
    const follower = await watchEntities(vi.fn(), onState);
    try {
      await vi.advanceTimersByTimeAsync(800);
      expect(closeFollower).toHaveBeenCalledOnce();
      onState.mockClear();
      staleState?.('offline', 'closed');
      await vi.advanceTimersByTimeAsync(0);
      expect(onState).not.toHaveBeenCalled();
    } finally {
      follower.close();
      leader.close();
    }
  });
});

// ---------------------------------------------------------------------------
// Lifecycle tests
// ---------------------------------------------------------------------------

describe('watchEntities lifecycle', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // Replace BroadcastChannel so every watchEntities call self-elects leader.
    vi.stubGlobal('BroadcastChannel', NoopBroadcastChannel);
  });

  afterEach(async () => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.resetAllMocks();
  });

  it('happy-path: emits connecting then live and delivers a KV tick', async () => {
    const { conn } = makeNatsConnMock([{ key: 'orders/placed/1', operation: 'PUT', revision: 7 }]);
    const { connect } = await import('nats.ws');
    (connect as ReturnType<typeof vi.fn>).mockResolvedValue(conn);
    stubFetch(null);

    const { watchEntities } = await import('./realtime');
    const states: string[] = [];
    const changes: KVChange[] = [];

    const handle = await watchEntities(
      (change) => changes.push(change),
      (state) => states.push(state),
    );

    // Advance the heartbeat timer once so becomeLeader fires, then drain microtasks.
    await vi.advanceTimersByTimeAsync(800);
    // Drain remaining microtask queue so the async KV loop finishes its entries.
    for (let i = 0; i < 10; i++) await Promise.resolve();

    expect(states).toContain('connecting');
    expect(states).toContain('live');
    expect(changes).toHaveLength(1);
    expect(changes[0]).toMatchObject({ key: 'orders/placed/1', operation: 'PUT', revision: '7' });

    handle.close();
  });

  it('close() stops the heartbeat interval and closes the NATS connection', async () => {
    const { conn, closeMock } = makeNatsConnMock([]);
    const { connect } = await import('nats.ws');
    (connect as ReturnType<typeof vi.fn>).mockResolvedValue(conn);
    stubFetch(null);

    const { watchEntities } = await import('./realtime');
    const handle = await watchEntities(vi.fn(), vi.fn());

    await vi.advanceTimersByTimeAsync(800);
    for (let i = 0; i < 10; i++) await Promise.resolve();

    const clearIntervalSpy = vi.spyOn(window, 'clearInterval');
    handle.close();

    expect(clearIntervalSpy).toHaveBeenCalled();
    expect(closeMock).toHaveBeenCalled();
  });

  it.each([
    [
      {
        type: 'error',
        data: 'PERMISSIONS_VIOLATION',
        permissionContext: { operation: 'publish', subject: '$JS.FC.KV_trogon-atlas-entities.consumer.token' },
      },
      'Permission denied: publish $JS.FC.KV_trogon-atlas-entities.consumer.token',
    ],
    [{ type: 'error', data: 'AUTHORIZATION_VIOLATION' }, 'AUTHORIZATION_VIOLATION'],
  ])('reports the connection status reason for %j', async (status, expectedReason) => {
    const { conn } = makeNatsConnMock();
    const { connect } = await import('nats.ws');
    (connect as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...conn,
      async *status() {
        yield status;
      },
    });
    stubFetch('test-token');
    const { watchEntities } = await import('@/lib/realtime');
    const onState = vi.fn();
    const handle = await watchEntities(vi.fn(), onState);
    try {
      await vi.advanceTimersByTimeAsync(0);
      expect(onState).toHaveBeenCalledWith('offline', expectedReason);
    } finally {
      handle.close();
    }
  });

  it('reconnect backoff: failed connect emits offline and retries after delay', async () => {
    const connectErr = new Error('dial failed');
    const { connect } = await import('nats.ws');
    // Fail twice then succeed on the third attempt.
    const { conn } = makeNatsConnMock([]);
    (connect as ReturnType<typeof vi.fn>)
      .mockRejectedValueOnce(connectErr)
      .mockRejectedValueOnce(connectErr)
      .mockResolvedValue(conn);
    stubFetch(null);

    const { watchEntities } = await import('./realtime');
    const states: string[] = [];
    const handle = await watchEntities(vi.fn(), (state) => states.push(state));

    // Advance enough time to cover two backoff sleeps (each up to 500ms at attempt=0)
    // plus the heartbeat tick that triggers becomeLeader.
    await vi.advanceTimersByTimeAsync(5000);
    for (let i = 0; i < 20; i++) await Promise.resolve();

    // Should have seen connecting, then offline for each failed attempt, then live.
    expect(states[0]).toBe('connecting');
    expect(states.filter((s) => s === 'offline').length).toBeGreaterThanOrEqual(2);
    expect(states).toContain('live');

    handle.close();
  });

  it('nats-auth failure does not claim permanent leadership without retry', async () => {
    // If /api/nats-auth fails once, watchEntitiesUnshared returns a noop
    // handle. becomeLeader must not treat that as lasting leadership;
    // otherwise the tab keeps heartbeating as leader, never re-dials, and
    // blocks peer tabs from taking over.
    const { connect } = await import('nats.ws');
    const connectMock = connect as ReturnType<typeof vi.fn>;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: false,
        status: 503,
        statusText: 'Service Unavailable',
        json: async () => ({}),
      } as Response),
    );

    const { watchEntities } = await import('./realtime');
    const states: string[] = [];
    const handle = await watchEntities(vi.fn(), (state) => states.push(state));

    await vi.advanceTimersByTimeAsync(800);
    for (let i = 0; i < 10; i++) await Promise.resolve();
    expect(states).toContain('offline');
    expect(connectMock).not.toHaveBeenCalled();

    // After auth recovers, a later heartbeat/checkLeader cycle should retry.
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ token: null }),
      } as Response),
    );
    const { conn } = makeNatsConnMock([]);
    connectMock.mockResolvedValue(conn);

    await vi.advanceTimersByTimeAsync(5_000);
    for (let i = 0; i < 20; i++) await Promise.resolve();

    expect(connectMock).toHaveBeenCalled();
    expect(states).toContain('live');
    handle.close();
  });

  it('watch loop stops delivering changes after close()', async () => {
    // A watch generator that never terminates on its own; we verify that
    // after close() the onChange callback stops being invoked even if the
    // underlying generator keeps producing entries (simulated via the
    // closed flag check inside the for-await loop).
    let resolveNext: ((v: IteratorResult<KvEntry>) => void) | undefined;
    async function* infiniteWatchGen(): AsyncGenerator<KvEntry> {
      while (true) {
        const result = await new Promise<IteratorResult<KvEntry>>((res) => {
          resolveNext = res;
        });
        if (result.done) return;
        yield result.value;
      }
    }

    async function* statusGen() {}
    const closeMock = vi.fn().mockResolvedValue(undefined);
    const conn = {
      close: closeMock,
      status: () => statusGen(),
      jetstream: () => ({
        views: {
          kv: async () => ({ watch: async () => infiniteWatchGen() }),
        },
      }),
    };

    const { connect } = await import('nats.ws');
    (connect as ReturnType<typeof vi.fn>).mockResolvedValue(conn);
    stubFetch(null);

    const { watchEntities } = await import('./realtime');
    const changes: KVChange[] = [];
    const handle = await watchEntities((change) => changes.push(change), vi.fn());

    await vi.advanceTimersByTimeAsync(800);
    for (let i = 0; i < 10; i++) await Promise.resolve();

    // Deliver one entry before close: should be received.
    resolveNext?.({ done: false, value: { key: 'a/b', operation: 'PUT', revision: 1 } });
    for (let i = 0; i < 10; i++) await Promise.resolve();
    expect(changes).toHaveLength(1);

    // Close the watch handle.
    handle.close();
    for (let i = 0; i < 10; i++) await Promise.resolve();

    // Any further entries must not reach onChange because closed=true.
    resolveNext?.({ done: false, value: { key: 'c/d', operation: 'PUT', revision: 2 } });
    for (let i = 0; i < 10; i++) await Promise.resolve();
    expect(changes).toHaveLength(1);
  });
});

describe('KVChange interface contract', () => {
  it('revision is always a string (bigint coercion contract)', () => {
    // Simulates the coercion done inside watchEntitiesUnshared before calling onChange.
    const bigintRevision: bigint = BigInt('9007199254740993'); // > Number.MAX_SAFE_INTEGER
    const numberRevision = 42;
    const fromBigint: KVChange = { key: 'test-key', operation: 'PUT', revision: String(bigintRevision) };
    const fromNumber: KVChange = { key: 'other-key', operation: 'DEL', revision: String(numberRevision) };
    expect(fromBigint.revision).toBe('9007199254740993');
    expect(fromNumber.revision).toBe('42');
    expect(typeof fromBigint.revision).toBe('string');
    expect(typeof fromNumber.revision).toBe('string');
  });

  it('satisfies the RealtimeWatch close() shape', () => {
    // Verify the interface is callable without runtime error.
    const noop: RealtimeWatch = { close: () => {} };
    expect(() => noop.close()).not.toThrow();
  });
});

describe('nextBackoffMs constants (inferred from spec)', () => {
  // nextBackoffMs(attempt) = Math.random() * Math.min(500 * 2^attempt, 30000)
  // We replicate the formula with the documented constants and verify bounds.
  const BACKOFF_BASE_MS = 500;
  const BACKOFF_MAX_MS = 30_000;

  function nextBackoffMs(attempt: number): number {
    const exp = Math.min(BACKOFF_BASE_MS * 2 ** attempt, BACKOFF_MAX_MS);
    return Math.random() * exp;
  }

  it('attempt=0 yields a value in [0, 500)', () => {
    for (let i = 0; i < 20; i++) {
      expect(nextBackoffMs(0)).toBeGreaterThanOrEqual(0);
      expect(nextBackoffMs(0)).toBeLessThan(BACKOFF_BASE_MS);
    }
  });

  it('caps at BACKOFF_MAX_MS regardless of large attempt count', () => {
    for (let i = 0; i < 20; i++) {
      expect(nextBackoffMs(100)).toBeLessThan(BACKOFF_MAX_MS);
    }
  });

  it('attempt=6 reaches the cap (500 * 2^6 = 32000 > 30000)', () => {
    // At attempt=6 the exponent would be 32000 which exceeds 30000, so the
    // formula should clamp. Verify the formula clamps correctly.
    const exp = Math.min(BACKOFF_BASE_MS * 2 ** 6, BACKOFF_MAX_MS);
    expect(exp).toBe(BACKOFF_MAX_MS);
  });
});

// ---------------------------------------------------------------------------
// Transport selection
//
// The NATS listener admits every client on the same unscoped grant, so a
// deployment that scopes namespaces per principal cannot use it. The bridge
// makes that call and the client obeys it.
// ---------------------------------------------------------------------------

describe('realtime transport selection', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
    vi.stubGlobal('BroadcastChannel', NoopBroadcastChannel);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.doUnmock('@/lib/change-stream');
    vi.resetModules();
    vi.clearAllMocks();
  });

  /** Drive the leader heartbeat so becomeLeader() runs, then drain microtasks. */
  async function settle() {
    await vi.advanceTimersByTimeAsync(800);
    for (let i = 0; i < 10; i++) await Promise.resolve();
  }

  it('never dials NATS when the bridge answers sse', async () => {
    const viaSse = vi.fn().mockReturnValue({ close: vi.fn() });
    vi.doMock('@/lib/change-stream', () => ({ watchChangesViaSse: viaSse }));
    const { connect } = await import('nats.ws');
    // A dial that would succeed, so choosing NATS here fails the
    // assertions rather than hanging the retry loop.
    (connect as ReturnType<typeof vi.fn>).mockResolvedValue(makeNatsConnMock().conn);
    stubFetch(null, 'sse');
    const { watchEntities } = await import('./realtime');
    const handle = await watchEntities(vi.fn(), vi.fn());
    await settle();
    expect(viaSse).toHaveBeenCalledTimes(1);
    expect(connect).not.toHaveBeenCalled();
    handle.close();
  });

  it('withholds the NATS transport even when a token comes back with it', async () => {
    // A bridge that says sse must be believed whatever else the body carries;
    // otherwise a stale token field would reopen the unscoped channel.
    const viaSse = vi.fn().mockReturnValue({ close: vi.fn() });
    vi.doMock('@/lib/change-stream', () => ({ watchChangesViaSse: viaSse }));
    const { connect } = await import('nats.ws');
    // A dial that would succeed, so choosing NATS here fails the
    // assertions rather than hanging the retry loop.
    (connect as ReturnType<typeof vi.fn>).mockResolvedValue(makeNatsConnMock().conn);
    stubFetch('leftover-token', 'sse');
    const { watchEntities } = await import('./realtime');
    const handle = await watchEntities(vi.fn(), vi.fn());
    await settle();
    expect(viaSse).toHaveBeenCalledTimes(1);
    expect(connect).not.toHaveBeenCalled();
    handle.close();
  });

  it('uses NATS when the bridge says nats', async () => {
    const viaSse = vi.fn();
    vi.doMock('@/lib/change-stream', () => ({ watchChangesViaSse: viaSse }));
    const { connect } = await import('nats.ws');
    const { conn } = makeNatsConnMock();
    (connect as ReturnType<typeof vi.fn>).mockResolvedValue(conn);
    stubFetch('ws-token', 'nats');
    const { watchEntities } = await import('./realtime');
    const handle = await watchEntities(vi.fn(), vi.fn());
    await settle();
    expect(viaSse).not.toHaveBeenCalled();
    expect(connect).toHaveBeenCalled();
    handle.close();
  });

  it('takes the bridge stream on a plain http orb.local page', async () => {
    // NATS admits the orb.local studio over https only, so dialing from
    // http would be refused at the handshake and leave the feed offline.
    vi.stubGlobal('location', new URL('http://trogon-atlas-studio.trogon-atlas.orb.local/'));
    const viaSse = vi.fn().mockReturnValue({ close: vi.fn() });
    vi.doMock('@/lib/change-stream', () => ({ watchChangesViaSse: viaSse }));
    const { connect } = await import('nats.ws');
    (connect as ReturnType<typeof vi.fn>).mockResolvedValue(makeNatsConnMock().conn);
    stubFetch(null, 'nats');
    const { watchEntities } = await import('./realtime');
    const handle = await watchEntities(vi.fn(), vi.fn());
    await settle();
    expect(viaSse).toHaveBeenCalledTimes(1);
    expect(connect).not.toHaveBeenCalled();
    handle.close();
  });

  it('dials NATS over wss on an https orb.local page', async () => {
    vi.stubGlobal('location', new URL('https://trogon-atlas-studio.trogon-atlas.orb.local/'));
    const viaSse = vi.fn();
    vi.doMock('@/lib/change-stream', () => ({ watchChangesViaSse: viaSse }));
    const { connect } = await import('nats.ws');
    (connect as ReturnType<typeof vi.fn>).mockResolvedValue(makeNatsConnMock().conn);
    stubFetch(null, 'nats');
    const { watchEntities } = await import('./realtime');
    const handle = await watchEntities(vi.fn(), vi.fn());
    await settle();
    expect(viaSse).not.toHaveBeenCalled();
    expect(connect).toHaveBeenCalledWith(expect.objectContaining({ servers: ['wss://nats.trogon-atlas.orb.local'] }));
    handle.close();
  });

  it('treats a bridge that names no transport as nats', async () => {
    const viaSse = vi.fn();
    vi.doMock('@/lib/change-stream', () => ({ watchChangesViaSse: viaSse }));
    const { connect } = await import('nats.ws');
    const { conn } = makeNatsConnMock();
    (connect as ReturnType<typeof vi.fn>).mockResolvedValue(conn);
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: true, json: async () => ({ token: 'ws-token' }) } as Response),
    );
    const { watchEntities } = await import('./realtime');
    const handle = await watchEntities(vi.fn(), vi.fn());
    await settle();
    expect(viaSse).not.toHaveBeenCalled();
    expect(connect).toHaveBeenCalled();
    handle.close();
  });
});

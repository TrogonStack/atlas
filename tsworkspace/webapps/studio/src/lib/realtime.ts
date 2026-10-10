// Realtime change feed via nats.ws: direct browser-to-NATS WebSocket
// connection. Watches the trogon-atlas-entities KV bucket and emits a
// notification whenever any key changes. The studio refetches the
// active model on each tick.
//
// Anonymous connection, always: the listener grants one identical read of
// the entity bucket to every client it admits, so authenticating it with
// NATS_WS_AUTH_TOKEN stops a drive-by without making it tenant-safe, and a
// credential that cannot be scoped is one the bridge will not publish. A
// deployment that sets either that token or a bridge credential is told so
// by `/api/nats-auth` answering `transport: 'sse'`; see
// `@/lib/change-stream`.
//
// Per-principal NATS auth is the documented follow-up
// (devops/docker/compose/dev/services/nats/nats-server.conf has the deployment posture;
// strategic-backlog §5 has the ruling). When that trigger fires the client
// side becomes: fetch a short-lived JWT from `/api/nats-token`, pass it to
// `connect({ authenticator: jwtAuthenticator(jwt) })`. No other call sites
// change.
//
// Decision #19 still holds at the gRPC layer: mutations go through the
// gateway → server → store. This is a read-side push channel; nothing
// is written here.
import { connect, type NatsConnection } from 'nats.ws';
import { watchChangesViaSse } from '@/lib/change-stream';
import { authHeaders } from '@/lib/credential';

// `null` means the listener cannot admit this page, so the feed goes through
// the bridge's stream instead.
function resolveWsUrl(): string | null {
  if (import.meta.env.VITE_NATS_WS_URL) return String(import.meta.env.VITE_NATS_WS_URL);
  // OrbStack publishes each compose service at <service>.<project>.orb.local.
  // The studio is served from trogon-atlas-studio.trogon-atlas.orb.local;
  // the NATS container lives next to it at nats.trogon-atlas.orb.local
  // and exposes its WS listener on container port 8080.
  if (typeof window === 'undefined') return 'ws://localhost:8080';
  const host = window.location.hostname;
  if (host.endsWith('.trogon-atlas.orb.local')) {
    // Over https the browser blocks insecure ws:// as mixed content, which
    // left the realtime channel permanently offline. OrbStack's TLS proxy
    // on 443 forwards to the NATS WS listener, so use wss without a port.
    // NATS keeps one allowed origin per hostname, and this one is the https
    // origin, so a plain http page would be refused at the handshake.
    return window.location.protocol === 'https:' ? 'wss://nats.trogon-atlas.orb.local' : null;
  }
  // Dev/localhost: the docker-compose publishes 8080 on the host.
  return `ws://${host}:8080`;
}

const WS_URL = resolveWsUrl();

const BUCKET = 'trogon-atlas-entities';

// Cap on a single KV entry the client will accept. Realtime is a hint to
// refetch, not a data channel, so a large value is almost certainly a
// misconfiguration or a hostile producer. The check is best-effort: not
// every KV variant exposes a `.value`/`.length` field, and we only act
// when we can read one cheaply.
const MAX_ENTRY_BYTES = 1_048_576; // 1 MiB

// Bounded exponential backoff with full jitter, used both for the initial
// connect and any reconnect attempts. Hard cap at ~30 s keeps the client
// polite to a recovering broker but eventually still tries every minute.
const BACKOFF_BASE_MS = 500;
const BACKOFF_MAX_MS = 30_000;
function nextBackoffMs(attempt: number): number {
  const exp = Math.min(BACKOFF_BASE_MS * 2 ** attempt, BACKOFF_MAX_MS);
  return Math.random() * exp;
}

export interface RealtimeWatch {
  close(): void;
}

export type RealtimeState = 'connecting' | 'live' | 'offline';
export type RealtimeStatus = { state: RealtimeState; detail?: string };

export interface KVChange {
  key: string;
  // 'PUT' or 'DEL' from the JetStream KV operation flag.
  operation: string;
  // nats.ws yields bigint or number depending on platform JS engine; we
  // coerce to a base-10 string at the boundary so any consumer can pass
  // the value through `JSON.stringify` (bigint would throw) or compare it
  // textually without worrying about precision.
  revision: string;
}

// JetStream KV entry as nats.ws exposes it. We narrow only the fields we
// touch; the real type carries more (created, delta, …) but we don't need
// them here.
interface KvEntry {
  key: string;
  operation: string;
  revision: bigint | number;
  length?: number;
  value?: Uint8Array;
}

interface KvLike {
  watch(opts: { ignoreDeletes: boolean }): Promise<AsyncIterable<KvEntry>>;
}

/**
 * Subscribe to KV changes on the entities bucket. The callback fires
 * once per key mutation. Returns a handle to close the watch.
 *
 * Multi-tab coordination: by default, only one tab per origin opens an
 * actual WebSocket to NATS. Other tabs receive the same change stream
 * via a `BroadcastChannel`, dropping N tabs of duplicate connections
 * down to one. Tabs elect a leader via a heartbeat on the channel; if
 * the current leader closes the tab or its heartbeat lapses, the next
 * `connecting`-state caller takes over.
 */
const COORDINATION_CHANNEL = 'trogon-atlas-studio:realtime';
const LEADER_HEARTBEAT_MS = 750;
const LEADER_STALE_MS = LEADER_HEARTBEAT_MS * 3;

type CoordinationMessage =
  | ({ type: 'heartbeat'; tabId: string; t: number } & RealtimeStatus)
  | { type: 'change'; tabId: string; change: KVChange }
  | ({ type: 'state'; tabId: string } & RealtimeStatus);

export async function watchEntities(
  onChange: (change: KVChange) => void,
  onState: (state: RealtimeState, detail?: string) => void,
): Promise<RealtimeWatch> {
  // BroadcastChannel is supported everywhere we ship (modern Chromium,
  // Firefox, Safari). When it isn't, fall back to the unshared path.
  if (typeof BroadcastChannel === 'undefined') {
    return watchEntitiesUnshared(onChange, onState);
  }

  const tabId =
    typeof crypto !== 'undefined' && 'randomUUID' in crypto
      ? crypto.randomUUID()
      : `${Math.random().toString(36).slice(2)}-${Date.now()}`;
  const channel = new BroadcastChannel(COORDINATION_CHANNEL);
  let leaderTabId: string | undefined;
  let leaderHeartbeatAt = 0;
  let leadership: RealtimeWatch | undefined;
  // Track an in-flight `becomeLeader()` so a second `checkLeader` tick
  // during the await window does not start a parallel NATS connection
  // (one of which would then leak when the second resolves).
  let electionInProgress = false;
  let closed = false;
  let currentStatus: RealtimeStatus | undefined;

  const reportState = (state: RealtimeState, detail?: string) => {
    if (closed) return;
    currentStatus = { state, detail };
    onState(state, detail);
  };

  const becomeLeader = async () => {
    if (electionInProgress || leadership || closed) return;
    electionInProgress = true;
    leaderTabId = tabId;
    try {
      const inner = await watchEntitiesUnshared(
        (change) => {
          if (closed || leaderTabId !== tabId) return;
          onChange(change);
          channel.postMessage({ type: 'change', tabId, change } satisfies CoordinationMessage);
        },
        (state, detail) => {
          if (closed || leaderTabId !== tabId) return;
          reportState(state, detail);
          channel.postMessage({
            type: 'state',
            tabId,
            state,
            detail,
          } satisfies CoordinationMessage);
        },
      );
      if (closed || leaderTabId !== tabId) {
        // Tab unmounted during the dial; close the inner connection
        // immediately rather than leak it.
        inner.close();
        return;
      }
      // Only claim leadership after a live connection is established.
      // Auth/connect failures throw (or never reach here), so checkLeader
      // can retry and peer tabs are not blocked by a noop holder.
      leadership = inner;
    } catch (err) {
      // Clear our tentative claim so the next heartbeat can re-elect.
      if (leaderTabId === tabId) leaderTabId = undefined;
      throw err;
    } finally {
      electionInProgress = false;
    }
  };

  const checkLeader = () => {
    if (closed || leadership || electionInProgress) return;
    const now = Date.now();
    if (!leaderTabId || now - leaderHeartbeatAt > LEADER_STALE_MS) {
      becomeLeader().catch((err: unknown) => {
        console.warn('trogon-atlas realtime: becomeLeader failed', err);
        if (!leaderTabId || leaderTabId === tabId) {
          reportState('offline', err instanceof Error ? err.message : String(err));
        }
      });
    }
  };

  const heartbeat = window.setInterval(() => {
    if (closed) return;
    if (leadership) {
      channel.postMessage({
        type: 'heartbeat',
        tabId,
        t: Date.now(),
        state: currentStatus?.state ?? 'connecting',
        detail: currentStatus?.detail,
      } satisfies CoordinationMessage);
      leaderHeartbeatAt = Date.now();
    } else {
      checkLeader();
    }
  }, LEADER_HEARTBEAT_MS);

  channel.onmessage = (event: MessageEvent<CoordinationMessage>) => {
    if (closed) return;
    const msg = event.data;
    if (msg.type === 'heartbeat') {
      if (msg.tabId === tabId) return;
      if ((leadership || electionInProgress) && msg.tabId > tabId) return;
      if (leaderTabId && msg.tabId > leaderTabId && Date.now() - leaderHeartbeatAt <= LEADER_STALE_MS) return;
      leaderTabId = msg.tabId;
      // Stamp the heartbeat against our own clock so leader staleness
      // detection isn't sensitive to skew between tabs (VM snapshots,
      // manual clock changes). The sender's `msg.t` is diagnostic only.
      leaderHeartbeatAt = Date.now();
      // Tiebreaker: the tab with the lexicographically smaller id wins.
      // Without this strict ordering two tabs could simultaneously
      // observe the other as the winner and both yield (deadlock).
      if (leadership && msg.tabId < tabId) {
        const previous = leadership;
        leadership = undefined;
        previous.close();
      }
      if (msg.state && (currentStatus?.state !== msg.state || currentStatus.detail !== msg.detail)) {
        reportState(msg.state, msg.detail);
      }
    } else if (msg.type === 'change') {
      if (!leadership && msg.tabId === leaderTabId) onChange(msg.change);
    } else if (msg.type === 'state') {
      if (!leadership && msg.tabId === leaderTabId) reportState(msg.state, msg.detail);
    }
  };

  // First check happens immediately so the only-tab case doesn't wait
  // a full heartbeat interval before connecting.
  checkLeader();

  return {
    close() {
      closed = true;
      window.clearInterval(heartbeat);
      leadership?.close();
      channel.close();
    },
  };
}

type Transport = 'nats' | 'sse';

type FetchNatsTokenResult = { ok: true; token: string | null; transport: Transport } | { ok: false; error: string };

async function fetchNatsToken(): Promise<FetchNatsTokenResult> {
  try {
    const res = await fetch('/api/nats-auth', { headers: authHeaders() });
    if (!res.ok) {
      return { ok: false, error: `nats-auth: ${res.status} ${res.statusText}` };
    }
    const body = (await res.json()) as { token?: string | null; transport?: string };
    return {
      ok: true,
      token: typeof body.token === 'string' ? body.token : null,
      // A bridge that predates the transport field only ever offered NATS,
      // and it is the one that answers without it.
      transport: body.transport === 'sse' ? 'sse' : 'nats',
    };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

async function watchEntitiesUnshared(
  onChange: (change: KVChange) => void,
  onState: (state: RealtimeState, detail?: string) => void,
): Promise<RealtimeWatch> {
  onState('connecting');
  const tokenResult = await fetchNatsToken();
  if (!tokenResult.ok) {
    onState('offline', tokenResult.error);
    // Throw so the multi-tab leader path does not treat a failed auth as
    // lasting leadership (a noop close handle would block retry and peer
    // takeover). Callers retry on the next election tick.
    throw new Error(tokenResult.error);
  }
  if (tokenResult.transport === 'sse') {
    // Not a degraded mode: the server decided this deployment's realtime
    // feed has to be authorized per caller, and this is that feed.
    return watchChangesViaSse(onChange, onState);
  }
  if (WS_URL === null) return watchChangesViaSse(onChange, onState);
  const natsToken = tokenResult.token;
  if (natsToken === null) {
    // Warn once per connection attempt so operators running a network-exposed
    // studio notice the unauthenticated posture without having to read the
    // NATS server logs. The connection proceeds: anonymous mode is intentional
    // for loopback deployments (see the module-level comment).
    //
    // The fix is a credential on the bridge, not on this listener: no
    // arrangement of `/api/nats-auth` hands out a NATS credential, so
    // reaching this line means the listener is genuinely open. Giving the
    // bridge a credential moves the feed to the per-caller stream; setting
    // NATS_WS_AUTH_TOKEN moves it there too, without the per-caller part.
    console.warn(
      '[trogon-atlas realtime] NATS WebSocket is running unauthenticated (server returned no token). ' +
        'This is safe for loopback deployments; for a network-exposed one, give the bridge a ' +
        'credential (TROGON_ATLAS_AUTH_TOKEN or TROGON_ATLAS_AUTH_PASSTHROUGH), which moves the feed ' +
        'to the per-caller stream.',
    );
  }
  // nats.ws throws a NatsError with an empty `.message` on transient
  // WS connect failures (e.g. during a studio rebuild). Build a
  // legible string so consumers don't print "NatsError: undefined".
  const describeErr = (err: unknown): string => {
    if (!(err instanceof Error)) return String(err);
    const code = (err as { code?: unknown }).code;
    const name = err.name || 'Error';
    const msg = err.message || (code ? `code=${String(code)}` : `connect to ${WS_URL} failed`);
    return `${name}: ${msg}`;
  };
  // Wrap nats.ws's built-in reconnect with our own exponential backoff:
  // 1. nats.ws supports a fixed `reconnectTimeWait`, not exponential, so a
  //    flapping broker today gets one reconnect attempt every 2 s forever.
  // 2. `waitOnFirstConnect: true` retries internally but with the same
  //    fixed interval. We loop the initial dial ourselves and apply
  //    jittered backoff up to ~30 s, then hand control to nats.ws's
  //    built-in reconnect for steady-state recovery.
  let nc: NatsConnection | undefined;
  let attempt = 0;
  // Cancellation flag set by close(); checked before each dial attempt
  // and after each backoff sleep so close() during retry stops dialing.
  let cancelled = false;
  const dialTimeoutRef: { current: ReturnType<typeof setTimeout> | undefined } = { current: undefined };
  while (!nc && !cancelled) {
    try {
      nc = await connect({
        servers: [WS_URL],
        reconnect: true,
        // Generous but finite: after 60 steady-state reconnect attempts
        // (~5 min at 5 s each) nats.ws gives up and we surface 'offline'.
        maxReconnectAttempts: 60,
        reconnectTimeWait: 5_000,
        // Use our own backoff loop for the initial dial; don't let nats.ws
        // hammer the broker every 2 s while we wait.
        waitOnFirstConnect: false,
        name: 'trogon-atlas-studio',
        ...(natsToken ? { token: natsToken } : {}),
      });
      // close() may have been called while connect() was in flight.
      if (cancelled) {
        nc.close().catch(() => {});
        return { close() {} };
      }
    } catch (err) {
      if (cancelled) return { close() {} };
      onState('offline', describeErr(err));
      const wait = nextBackoffMs(attempt);
      attempt = Math.min(attempt + 1, 6);
      await new Promise<void>((r) => {
        const t = setTimeout(r, wait);
        dialTimeoutRef.current = t;
      });
      if (cancelled) return { close() {} };
    }
  }
  if (!nc) return { close() {} };
  onState('live');

  let closed = false;

  // Forward connection status changes to the consumer so the UI dot can
  // reflect live/offline accurately instead of relying on tick latency.
  // Also surfaces reconnect-budget exhaustion (maxReconnectAttempts reached)
  // so the UI dot reflects the terminal offline state.
  (async () => {
    for await (const status of nc.status()) {
      if (closed) break;
      if (status.type === 'reconnecting') onState('offline', 'reconnecting');
      else if (status.type === 'reconnect') onState('live');
      else if (status.type === 'error') {
        const permission = status.permissionContext;
        const detail = permission
          ? `Permission denied: ${permission.operation} ${permission.subject}`
          : typeof status.data === 'string'
            ? status.data
            : status.type;
        onState('offline', detail);
      } else if (status.type === 'disconnect') onState('offline', status.type);
    }
  })().catch((err: unknown) => {
    console.warn('trogon-atlas realtime: status iterator failed', err);
    onState('offline', err instanceof Error ? err.message : String(err));
  });

  let watchIterator:
    | (AsyncIterable<KvEntry> & { return?: (value?: unknown) => Promise<IteratorResult<KvEntry>> })
    | undefined;
  (async () => {
    try {
      // nats.ws exposes KV through the JetStream client's views.
      if (typeof nc.jetstream !== 'function') {
        throw new Error('nats.ws: jetstream() not available on this connection; update the nats.ws package');
      }
      const js = nc.jetstream();
      const kv = (await js.views.kv(BUCKET)) as KvLike;
      const watch = await kv.watch({ ignoreDeletes: false });
      watchIterator = watch;
      for await (const entry of watch) {
        if (closed) break;
        // Cheap payload-size guard: realtime is only a refetch hint, so a
        // huge value almost certainly means a misconfiguration. Drop the
        // entry rather than forwarding it (the next listEntities call will
        // re-validate the data path anyway).
        const size =
          typeof entry.length === 'number'
            ? entry.length
            : entry.value instanceof Uint8Array
              ? entry.value.byteLength
              : 0;
        if (size > MAX_ENTRY_BYTES) {
          continue;
        }
        onChange({
          key: entry.key,
          operation: entry.operation,
          revision: String(entry.revision),
        });
      }
    } catch (err) {
      onState('offline', describeErr(err));
    }
  })();

  return {
    close() {
      closed = true;
      cancelled = true;
      if (dialTimeoutRef.current !== undefined) {
        clearTimeout(dialTimeoutRef.current);
        dialTimeoutRef.current = undefined;
      }
      watchIterator?.return?.();
      nc.close().catch(() => {});
    },
  };
}

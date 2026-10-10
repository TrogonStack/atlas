// Realtime change feed over the bridge's SSE endpoint, used when the direct
// NATS WebSocket is not an option.
//
// The NATS listener grants one unscoped read of the entity bucket to every
// client it admits: one shared token, one permission set, no notion of who
// is connected. A deployment that scopes namespaces per principal cannot use
// it, because it would hand every browser the entities the gRPC lens spent
// its effort hiding. `/api/changes/stream` has no such problem: it polls
// `listChanges` through the server, so it sees exactly what the caller's own
// credential allows.
//
// `EventSource` cannot carry an `Authorization` header, and in pass-through
// mode the stream requires one, so this reads the response body directly.
// That also means reconnect and `Last-Event-ID` resume are ours to implement
// rather than the browser's.
import { reportUnauthenticated } from '@/lib/api';
import { authHeaders } from '@/lib/credential';

export interface ChangeStreamHandle {
  close(): void;
}

/** Mirrors realtime.ts's KVChange so either transport feeds the same sink. */
export interface StreamChange {
  key: string;
  operation: string;
  revision: string;
}

type StreamState = 'connecting' | 'live' | 'offline';

const BACKOFF_BASE_MS = 500;
const BACKOFF_MAX_MS = 30_000;

function nextBackoffMs(attempt: number): number {
  const exp = Math.min(BACKOFF_BASE_MS * 2 ** attempt, BACKOFF_MAX_MS);
  return Math.random() * exp;
}

interface ChangeEventLike {
  kind?: string;
  token?: string;
  entity?: { kind?: string; id?: { namespace?: string; slug?: string; version?: string } };
}

/**
 * Best-effort key for a change event. The consumer refetches on any change
 * rather than reading this, so an event whose ref the server did not fill in
 * still counts as a change; it just reports an empty key.
 */
function changeKey(event: ChangeEventLike): string {
  const id = event.entity?.id;
  if (!id) return '';
  return [id.namespace, id.slug, id.version].filter(Boolean).join('.');
}

function toChange(event: ChangeEventLike, fallbackRevision: string): StreamChange {
  return {
    key: changeKey(event),
    operation: event.kind === 'KIND_DELETED' ? 'DEL' : 'PUT',
    revision: event.token || fallbackRevision,
  };
}

/**
 * Parse whatever whole SSE frames the buffer holds, returning the leftover.
 * A frame is terminated by a blank line, so a partial trailing frame stays
 * in the buffer until the next read completes it.
 */
export function parseFrames(buffer: string): {
  frames: Array<{ event: string; data: string; id: string }>;
  rest: string;
} {
  const frames: Array<{ event: string; data: string; id: string }> = [];
  const parts = buffer.split('\n\n');
  const rest = parts.pop() ?? '';
  for (const part of parts) {
    let event = 'message';
    let id = '';
    const data: string[] = [];
    for (const line of part.split('\n')) {
      if (line.startsWith(':')) continue;
      if (line.startsWith('event:')) event = line.slice(6).trim();
      else if (line.startsWith('id:')) id = line.slice(3).trim();
      else if (line.startsWith('data:')) data.push(line.slice(5).trim());
    }
    if (data.length > 0 || event !== 'message') {
      frames.push({ event, data: data.join('\n'), id });
    }
  }
  return { frames, rest };
}

/**
 * Follow the bridge's change stream, calling `onChange` once per change event.
 * Reconnects with jittered backoff, resuming from the last id it saw.
 */
export function watchChangesViaSse(
  onChange: (change: StreamChange) => void,
  onState: (state: StreamState, detail?: string) => void,
): ChangeStreamHandle {
  let closed = false;
  let lastEventId = '';
  const controllers = new Set<AbortController>();

  const sleep = (ms: number) =>
    new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, ms);
      if (closed) {
        clearTimeout(timer);
        resolve();
      }
    });

  (async () => {
    let attempt = 0;
    while (!closed) {
      onState('connecting');
      const controller = new AbortController();
      controllers.add(controller);
      try {
        const headers: Record<string, string> = { ...authHeaders(), Accept: 'text/event-stream' };
        if (lastEventId) headers['Last-Event-ID'] = lastEventId;
        const res = await fetch('/api/changes/stream', { headers, signal: controller.signal });
        if (!res.ok || !res.body) {
          const message = `changes/stream: ${res.status} ${res.statusText}`;
          // The bridge vouches for the credential before it opens the
          // stream, so a refusal here is the key, not the feed.
          if (res.status === 401 || res.status === 403) reportUnauthenticated(message);
          throw new Error(message);
        }
        attempt = 0;
        onState('live');
        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        let buffer = '';
        let broke = '';
        while (!closed) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          const { frames, rest } = parseFrames(buffer);
          buffer = rest;
          for (const frame of frames) {
            if (frame.id) lastEventId = frame.id;
            // `fatal` is the bridge giving up; `error` is one upstream poll
            // that failed. Neither is survivable in place: the bridge only
            // writes on a change, so a stream that has started failing looks
            // exactly like a quiet one from here. Both end this connection
            // and let the reconnect find out which it was, which is also how
            // a credential revoked mid-stream becomes a 401 the caller sees
            // rather than a feed that silently stops delivering.
            if (frame.event === 'fatal' || frame.event === 'error') {
              broke = frame.data;
              break;
            }
            let payload: { events?: ChangeEventLike[] };
            try {
              payload = JSON.parse(frame.data);
            } catch {
              continue;
            }
            for (const event of payload.events ?? []) {
              onChange(toChange(event, frame.id));
            }
          }
          if (broke) break;
        }
        if (broke) throw new Error(broke);
        // The server ended the stream cleanly. Reconnect, but not instantly:
        // a server that closes on accept would otherwise spin here.
        if (!closed) await sleep(nextBackoffMs(0));
      } catch (err) {
        if (closed) break;
        onState('offline', err instanceof Error ? err.message : String(err));
        await sleep(nextBackoffMs(attempt));
        attempt += 1;
      } finally {
        controllers.delete(controller);
      }
    }
    if (!closed) onState('offline');
  })();

  return {
    close() {
      closed = true;
      for (const controller of controllers) controller.abort();
      controllers.clear();
    },
  };
}

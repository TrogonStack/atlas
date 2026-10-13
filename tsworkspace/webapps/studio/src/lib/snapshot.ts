// A snapshot is one board saved with the read responses it was drawn from,
// so the board can render later with no backend. It is a projection of the
// state at `capturedAt`, not an offline mode: nothing in it can refresh,
// and the UI says when it was taken.

export const SNAPSHOT_VERSION = 1;

export interface StudioSnapshot {
  version: typeof SNAPSHOT_VERSION;
  /** ISO-8601 instant the responses were read. */
  capturedAt: string;
  namespace: string;
  slug: string;
  /** Branch the board was read from; omitted for the baseline. */
  branch?: string;
  /** Response bodies keyed by the request path `api.ts` asked for. */
  responses: Record<string, unknown>;
}

let active: StudioSnapshot | undefined;

/** Answers every read from `snapshot` until the page unloads. */
export function installSnapshot(snapshot: StudioSnapshot): void {
  active = snapshot;
}

export function activeSnapshot(): StudioSnapshot | undefined {
  return active;
}

/** Test-only: return to normal network reads. */
export function clearSnapshot(): void {
  active = undefined;
}

/**
 * The recorded body for `path`. A path the snapshot does not hold throws,
 * so a view that needs more than the board was saved with shows an error
 * instead of quietly rendering empty.
 */
export function snapshotResponse(snapshot: StudioSnapshot, path: string): unknown {
  if (!Object.hasOwn(snapshot.responses, path)) {
    throw new Error(`not in this snapshot: ${path}`);
  }
  return snapshot.responses[path];
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

export function parseSnapshot(text: string): StudioSnapshot {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    throw new Error('snapshot data is not valid JSON');
  }
  if (!isRecord(raw)) throw new Error('snapshot data is not an object');
  if (raw.version !== SNAPSHOT_VERSION) {
    throw new Error(`unsupported snapshot version: ${String(raw.version)}`);
  }
  for (const field of ['capturedAt', 'namespace', 'slug'] as const) {
    if (typeof raw[field] !== 'string' || raw[field] === '') {
      throw new Error(`snapshot is missing ${field}`);
    }
  }
  if (raw.branch !== undefined && typeof raw.branch !== 'string') {
    throw new Error('snapshot branch must be a string');
  }
  if (!isRecord(raw.responses)) throw new Error('snapshot is missing responses');
  return raw as unknown as StudioSnapshot;
}

function filePart(s: string): string {
  return s.replace(/[^A-Za-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '');
}

/** `<slug>[-<branch>]-<YYYYMMDD-HHmm>.html`, in UTC so the name does not depend on who saved it. */
export function snapshotFileName(snapshot: Pick<StudioSnapshot, 'slug' | 'branch' | 'capturedAt'>): string {
  const at = new Date(snapshot.capturedAt);
  const pad = (n: number) => String(n).padStart(2, '0');
  const stamp = Number.isNaN(at.getTime())
    ? 'snapshot'
    : `${at.getUTCFullYear()}${pad(at.getUTCMonth() + 1)}${pad(at.getUTCDate())}-${pad(at.getUTCHours())}${pad(at.getUTCMinutes())}`;
  const parts = [filePart(snapshot.slug), snapshot.branch ? filePart(snapshot.branch) : '', stamp].filter(Boolean);
  return `${parts.join('-')}.html`;
}

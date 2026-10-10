/**
 * The browser's own API credential.
 *
 * The bridge used to hold a single token and make every upstream call as
 * that one principal, so two people looking at the same studio saw the same
 * namespaces no matter which key they held. With
 * `TROGON_ATLAS_AUTH_PASSTHROUGH=true` the bridge forwards whatever credential
 * the browser presents and the server decides what that principal may see,
 * which means the browser needs somewhere to keep one.
 *
 * `sessionStorage`, not `localStorage`: both are readable by script on this
 * origin, so neither defends against XSS, but a session-scoped copy dies
 * with the tab instead of outliving the person who typed it. In-memory only
 * would be safer still and would log you out on every refresh, which people
 * work around by pasting the key into somewhere worse.
 */

const STORAGE_KEY = 'trogon-atlas.credential';

let cached: string | null = null;
let loaded = false;

type Listener = () => void;
const listeners = new Set<Listener>();

function storage(): Storage | null {
  try {
    return window.sessionStorage;
  } catch {
    // Blocked by cookie policy or a sandboxed frame. Fall back to memory.
    return null;
  }
}

export function getCredential(): string | null {
  if (!loaded) {
    try {
      cached = storage()?.getItem(STORAGE_KEY) ?? null;
    } catch {
      cached = null;
    }
    loaded = true;
  }
  return cached;
}

export function setCredential(token: string): void {
  const trimmed = token.trim();
  cached = trimmed.length > 0 ? trimmed : null;
  loaded = true;
  try {
    if (cached) storage()?.setItem(STORAGE_KEY, cached);
    else storage()?.removeItem(STORAGE_KEY);
  } catch {
    // Memory-only for this tab.
  }
  notify();
}

export function clearCredential(): void {
  setCredential('');
}

/** Header for a request that should carry the browser's identity. */
export function authHeaders(): Record<string, string> {
  const token = getCredential();
  return token ? { Authorization: `Bearer ${token}` } : {};
}

/**
 * Called when the bridge or the server refuses the credential we sent. The
 * stored token is dropped, because a rejected credential that stays in
 * storage makes every subsequent request fail the same way with no visible
 * cause.
 */
export function onUnauthenticated(): void {
  if (getCredential() === null) {
    notify();
    return;
  }
  setCredential('');
}

export function subscribeCredential(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function notify(): void {
  for (const listener of [...listeners]) listener();
}

/** Test seam: forget what was read from storage. */
export function resetCredentialCacheForTests(): void {
  cached = null;
  loaded = false;
}

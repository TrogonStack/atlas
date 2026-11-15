import { useSyncExternalStore } from 'react';
import { z } from 'zod';

const studioFeatureFlagsSchema = z.object({
  laneMesh: z.boolean().catch(false),
  streamTowers: z.boolean().catch(false),
});

export type StudioFeatureFlags = Readonly<z.infer<typeof studioFeatureFlagsSchema>>;

export type StudioFeatureFlag = keyof StudioFeatureFlags;

const STORAGE_KEY = 'trogon-atlas.studio.feature-flags';
const DEFAULT_FLAGS: StudioFeatureFlags = Object.freeze({ laneMesh: false, streamTowers: false });
const listeners = new Set<() => void>();
let snapshot = DEFAULT_FLAGS;
let storedValue: string | null | undefined;
let pendingChanges: Partial<StudioFeatureFlags> = {};

function parseFlags(value: string | null): StudioFeatureFlags {
  try {
    const stored: unknown = JSON.parse(value ?? 'null');
    return studioFeatureFlagsSchema.parse(stored);
  } catch {
    return DEFAULT_FLAGS;
  }
}

function updateSnapshot(flags: StudioFeatureFlags) {
  if (snapshot.laneMesh !== flags.laneMesh || snapshot.streamTowers !== flags.streamTowers) {
    snapshot = Object.freeze(flags);
  }
}

function readStoredValue(value: string | null, externalChange = false) {
  if (externalChange && value === null) pendingChanges = {};
  if (value === storedValue && !externalChange) return;
  storedValue = value;
  updateSnapshot({ ...parseFlags(value), ...pendingChanges });
}

function refreshFromStorage() {
  try {
    if (typeof window !== 'undefined') readStoredValue(window.localStorage.getItem(STORAGE_KEY));
  } catch {
    // Keep the shared snapshot when browser storage is unavailable.
  }
}

function notify() {
  for (const listener of listeners) listener();
}

function onStorage(event: StorageEvent) {
  if (event.key !== null && event.key !== STORAGE_KEY) return;
  const previous = snapshot;
  try {
    const storage = window.localStorage;
    if (event.storageArea !== null && event.storageArea !== storage) return;
    readStoredValue(storage.getItem(STORAGE_KEY), true);
  } catch {
    return;
  }
  if (snapshot !== previous) notify();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1 && typeof window !== 'undefined') window.addEventListener('storage', onStorage);
  const previous = snapshot;
  refreshFromStorage();
  if (snapshot !== previous) notify();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && typeof window !== 'undefined') window.removeEventListener('storage', onStorage);
  };
}

function getSnapshot() {
  if (listeners.size === 0) refreshFromStorage();
  return snapshot;
}

function getServerSnapshot() {
  return DEFAULT_FLAGS;
}

function setFeatureFlag(feature: StudioFeatureFlag, enabled: boolean) {
  const previous = snapshot;
  refreshFromStorage();
  pendingChanges = { ...pendingChanges, [feature]: enabled };
  updateSnapshot({ ...snapshot, [feature]: enabled });
  try {
    const serialized = JSON.stringify(snapshot);
    window.localStorage.setItem(STORAGE_KEY, serialized);
    storedValue = serialized;
    pendingChanges = {};
  } catch {
    // Failed writes stay shared in memory instead of reverting to old storage.
  }
  if (snapshot !== previous) notify();
}

export function useStudioFeatureFlags() {
  const flags = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
  return [flags, setFeatureFlag] as const;
}

/** The per-tab store outlives hook mounts, so tests reset it after unmounting their consumers. */
export function resetFeatureFlagsForTests() {
  snapshot = DEFAULT_FLAGS;
  storedValue = undefined;
  pendingChanges = {};
}

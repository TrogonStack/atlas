import { useCallback, useEffect, useState } from 'react';

export interface StudioFeatureFlags {
  readonly laneMesh: boolean;
  readonly streamTowers: boolean;
}

export type StudioFeatureFlag = keyof StudioFeatureFlags;

const STORAGE_KEY = 'trogon-atlas.studio.feature-flags';
const DEFAULT_FLAGS: StudioFeatureFlags = { laneMesh: false, streamTowers: false };

function loadFeatureFlags(): StudioFeatureFlags {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? 'null');
    if (!stored || typeof stored !== 'object' || Array.isArray(stored)) return DEFAULT_FLAGS;
    const flags = stored as Record<string, unknown>;
    return { laneMesh: flags.laneMesh === true, streamTowers: flags.streamTowers === true };
  } catch {
    return DEFAULT_FLAGS;
  }
}

export function useStudioFeatureFlags() {
  const [flags, setFlags] = useState(loadFeatureFlags);
  const setFeatureFlag = useCallback((feature: StudioFeatureFlag, enabled: boolean) => {
    setFlags((current) => ({ ...current, [feature]: enabled }));
  }, []);

  useEffect(() => {
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(flags));
    } catch {
      // Storage can be blocked or full; keep the preference for this page.
    }
  }, [flags]);

  return [flags, setFeatureFlag] as const;
}

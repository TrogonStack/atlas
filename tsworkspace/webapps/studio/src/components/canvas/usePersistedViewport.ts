import type { Viewport } from '@xyflow/react';
import { useReactFlow } from '@xyflow/react';
import { useCallback, useEffect } from 'react';

const PREFIX = 'trogon-atlas-studio:viewport:';

function readViewport(key: string): Viewport | undefined {
  try {
    const raw = window.sessionStorage.getItem(PREFIX + key);
    if (!raw) return undefined;
    const parsed = JSON.parse(raw) as Partial<Viewport>;
    if (
      typeof parsed.x !== 'number' ||
      typeof parsed.y !== 'number' ||
      typeof parsed.zoom !== 'number' ||
      !Number.isFinite(parsed.x) ||
      !Number.isFinite(parsed.y) ||
      !Number.isFinite(parsed.zoom)
    ) {
      return undefined;
    }
    return { x: parsed.x, y: parsed.y, zoom: parsed.zoom };
  } catch {
    return undefined;
  }
}

export function usePersistedViewport(key: string, enabled: boolean) {
  const { setViewport } = useReactFlow();
  // Derive synchronously from key so the first render after a key change
  // already reports restored=true when storage has a viewport (Board uses
  // restored during render for fitView={!restored}).
  const restored = enabled && Boolean(readViewport(key));

  useEffect(() => {
    if (!enabled) return;
    const viewport = readViewport(key);
    if (!viewport) return;
    const timer = window.setTimeout(() => {
      setViewport(viewport, { duration: 0 });
    }, 0);
    return () => window.clearTimeout(timer);
  }, [enabled, key, setViewport]);

  const onMoveEnd = useCallback(
    (_event: unknown, viewport: Viewport) => {
      if (!enabled) return;
      try {
        window.sessionStorage.setItem(PREFIX + key, JSON.stringify(viewport));
      } catch {
        return;
      }
    },
    [enabled, key],
  );

  return { onMoveEnd, restored };
}

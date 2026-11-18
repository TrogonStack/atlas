// The stream towers tab: each event stream as a tower of its events, with
// the commands that append to it and the read models that project from it.

import { useMemo, useRef } from 'react';
import type { ThreeBoardProps } from '@/components/canvas/ThreeView';
import { buildTowers } from '@/lib/towers';
import type { SceneSpec } from '@/lib/scene3d';
import { GraphBoard } from './GraphBoard';

export default function StreamTowersBoard({ model, ...props }: ThreeBoardProps) {
  // Every realtime event makes a new model. Reusing the previous spec when the content is equal stops the scene from repainting.
  const last = useRef<{ key: string; spec: SceneSpec }>(undefined);
  const built = useMemo(() => {
    const spec = buildTowers(model);
    return { spec, key: JSON.stringify(spec) };
  }, [model]);
  if (last.current?.key !== built.key) last.current = built;
  return <GraphBoard spec={last.current.spec} {...props} />;
}

// The lane mesh tab: the Board's lanes and slice columns in 3D, with links
// arcing off the board and the cited code under it.

import { useMemo, useRef } from 'react';
import type { ThreeBoardProps } from '@/components/canvas/ThreeView';
import { buildLaneMesh } from '@/lib/lanemesh';
import type { SceneSpec } from '@/lib/scene3d';
import { GraphBoard } from './GraphBoard';

export default function LaneMeshBoard({ model, ...props }: ThreeBoardProps) {
  // Every realtime event makes a new model. Reusing the previous spec when the content is equal stops the scene from repainting.
  const last = useRef<{ key: string; spec: SceneSpec }>(undefined);
  const built = useMemo(() => {
    const spec = buildLaneMesh(model);
    return { spec, key: JSON.stringify(spec) };
  }, [model]);
  if (last.current?.key !== built.key) last.current = built;
  return <GraphBoard spec={last.current.spec} {...props} />;
}

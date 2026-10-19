// The stream towers tab: each event stream as a tower of its events, with
// the commands that append to it and the read models that project from it.

import { useMemo } from 'react';
import type { ThreeBoardProps } from '@/components/canvas/ThreeView';
import { buildTowers } from '@/lib/towers';
import { GraphBoard } from './GraphBoard';

export default function StreamTowersBoard({ model, ...props }: ThreeBoardProps) {
  const spec = useMemo(() => buildTowers(model), [model]);
  return <GraphBoard spec={spec} {...props} />;
}

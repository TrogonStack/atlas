// Entry point for the 3D views. three.js is large, so the boards load only
// when a 3D tab opens: the Board and the main bundle do not carry it. A
// browser without WebGL gets a plain message and never downloads the 3D
// code at all.

import { Box } from 'lucide-react';
import { lazy, Suspense, useState } from 'react';
import type { SelectedEdge, Selection } from '@/components/canvas/Board';
import type { FocusRequest } from '@/components/canvas/useFocusNode';
import { BoardEmptyState } from '@/components/ui/empty-state';
import type { Model } from '@/lib/model';
import { hasWebGL } from '@/lib/webgl';

const LaneMeshBoard = lazy(() => import('@/components/canvas/three/LaneMeshBoard'));

export type ThreeViewName = 'lanes';

export interface ThreeBoardProps {
  model: Model;
  selections: Selection[];
  onSelect: (
    key: string | undefined,
    occId?: string,
    additive?: boolean,
    instance?: string,
    momentKey?: string,
  ) => void;
  onSelectEdge: (edge: SelectedEdge, additive?: boolean) => void;
  focus?: FocusRequest;
}

export function ThreeView({ view, ...props }: ThreeBoardProps & { view: ThreeViewName }) {
  // Probed once per mount: a browser does not gain WebGL while the page is open.
  const [webgl] = useState(hasWebGL);
  if (!webgl) {
    return (
      <div className="relative h-full w-full">
        <BoardEmptyState icon={Box} title="This view needs WebGL">
          This browser cannot draw 3D graphics, or WebGL is turned off. The other views work without it.
        </BoardEmptyState>
      </div>
    );
  }
  return (
    <Suspense
      fallback={
        <div className="relative h-full w-full">
          <BoardEmptyState icon={Box} title="Loading the 3D view" />
        </div>
      }
    >
      {view === 'lanes' ? <LaneMeshBoard {...props} /> : null}
    </Suspense>
  );
}

// Stands in for the 3D views in a saved board's runtime. A saved board never
// offers the 3D tabs, and the runtime is one inlined file, so leaving the 3D
// code and three.js out keeps every saved board smaller.

import { Box } from 'lucide-react';
import { BoardEmptyState } from '@/components/ui/empty-state';

export function ThreeView() {
  return (
    <div className="relative h-full w-full">
      <BoardEmptyState icon={Box} title="3D views are not part of a saved board" />
    </div>
  );
}

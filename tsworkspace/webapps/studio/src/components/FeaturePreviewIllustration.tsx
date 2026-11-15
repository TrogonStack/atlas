import { Box } from 'lucide-react';
import { lazy, Suspense, useState } from 'react';
import type { Selection } from '@/components/canvas/Board';
import { StateMachineBoard } from '@/components/canvas/StateMachineBoard';
import { ErrorBoundary } from '@/components/ErrorBoundary';
import { BoardEmptyState } from '@/components/ui/empty-state';
import type { StudioFeatureFlag } from '@/lib/feature-flags';
import { buildFeaturePreviewModel, buildStateMachinePreviewModel } from '@/lib/feature-preview-model';

const ThreeView = lazy(() => import('@/components/canvas/ThreeView').then((module) => ({ default: module.ThreeView })));
const EXAMPLE_MODEL = buildFeaturePreviewModel();
const LIFECYCLE_MODEL = buildStateMachinePreviewModel();

export function FeaturePreviewIllustration({ feature }: { feature: StudioFeatureFlag }) {
  const [selections, setSelections] = useState<Selection[]>([]);
  return (
    <figure className="shrink-0 overflow-hidden rounded-lg border border-border bg-background">
      <div className="relative h-[300px]">
        <ErrorBoundary key={feature}>
          {feature === 'stateMachine' ? (
            <StateMachineBoard
              model={LIFECYCLE_MODEL}
              selections={selections}
              onSelect={(key) => setSelections(key ? [{ type: 'entity', key }] : [])}
              fitWhole
            />
          ) : (
            <Suspense fallback={<BoardEmptyState icon={Box} title="Loading the 3D view" />}>
              <ThreeView
                view={feature === 'laneMesh' ? 'lanes' : 'towers'}
                model={EXAMPLE_MODEL}
                selections={selections}
                onSelect={(key, occId) => setSelections(key ? [{ type: 'entity', key, occId }] : [])}
                onSelectEdge={(edge) => setSelections([{ type: 'edge', id: edge.id, data: edge.data }])}
              />
            </Suspense>
          )}
        </ErrorBoundary>
      </div>
      <figcaption className="flex flex-wrap items-center justify-between gap-2 border-t border-border bg-card px-4 py-3 text-xs text-muted-foreground">
        <span>Example model</span>
        <span className="text-[11px]">
          {feature === 'stateMachine' ? 'Same view, fictional data' : 'Same 3D view, fictional data'}
        </span>
      </figcaption>
    </figure>
  );
}

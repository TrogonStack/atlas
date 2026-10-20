// The React shell around a GraphScene: selection in, clicks out, camera
// presets, the hover tip, and a plain message when WebGL fails. Each 3D
// view builds its own SceneSpec and hands it here, so the views share one
// behavior for picking, focus and disposal.

import { Box, ChevronLeft, ChevronRight } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { BoardTopRight } from '@/components/canvas/BoardTopRight';
import type { ThreeBoardProps } from '@/components/canvas/ThreeView';
import { Button } from '@/components/ui/button';
import { BoardEmptyState } from '@/components/ui/empty-state';
import { resolveFocus, type SceneSpec } from '@/lib/scene3d';
import { kindColors } from './palette';
import { GraphScene, type ScenePick } from './scene';

type Failure = 'unavailable' | 'lost';

// Inline, not a Tailwind class: a class only this view uses would grow the stylesheet every page loads.
const BACKDROP = 'radial-gradient(120% 90% at 50% 0%, #ffffff 0%, #f4f4f5 75%)';

function describe(pick: ScenePick): { title: string; detail: string } {
  if (pick.type === 'node') {
    const { entity, instance, synthetic } = pick.node;
    const parts = [kindColors(entity.kind).label, `${entity.id.namespace}/${entity.id.slug}`];
    if (instance) parts.push(instance);
    if (synthetic) parts.push('implicit: no slice shows this update');
    return { title: entity.title, detail: parts.join(' · ') };
  }
  if (pick.type === 'plate') {
    const { plate } = pick;
    return { title: plate.title, detail: plate.kicker ?? (plate.kind ? kindColors(plate.kind).label : '') };
  }
  const { file } = pick;
  const cites = `cited by ${file.citedBy.length} ${file.citedBy.length === 1 ? 'entity' : 'entities'}`;
  return { title: file.path, detail: [file.repo, ...file.symbols, cites].join(' · ') };
}

export function GraphBoard({
  spec,
  selections,
  onSelect,
  focus,
}: Omit<ThreeBoardProps, 'model'> & { spec: SceneSpec }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<GraphScene | undefined>(undefined);
  const builtRef = useRef<SceneSpec | undefined>(undefined);
  const [failure, setFailure] = useState<Failure>();
  const [step, setStep] = useState<number>();
  const focusState = useMemo(() => resolveFocus(spec, selections), [spec, selections]);

  // Read through refs so the scene is created once, not on every new callback or selection.
  const onSelectRef = useRef(onSelect);
  onSelectRef.current = onSelect;
  const specRef = useRef(spec);
  specRef.current = spec;
  const focusStateRef = useRef(focusState);
  focusStateRef.current = focusState;
  const selectionsRef = useRef(selections);
  selectionsRef.current = selections;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let scene: GraphScene | undefined;
    try {
      scene = new GraphScene(host, {
        pick: (pick, additive) => {
          if (!pick) onSelectRef.current(undefined);
          else if (pick.type === 'node') {
            const n = pick.node;
            onSelectRef.current(n.entity.key, n.occId, additive, n.instanceUid, n.momentKey);
          } else if (pick.type === 'plate' && pick.plate.entity) {
            onSelectRef.current(pick.plate.entity.key, undefined, additive);
          }
          // A code file is not a model entity, so a click on one keeps the selection.
        },
        hover: (pick, at) => {
          const tip = tipRef.current;
          if (!tip) return;
          if (!pick || !at) {
            tip.hidden = true;
            return;
          }
          const { title, detail } = describe(pick);
          const box = host.getBoundingClientRect();
          (tip.children[0] as HTMLElement).textContent = title;
          (tip.children[1] as HTMLElement).textContent = detail;
          tip.hidden = false;
          tip.style.left = `${Math.min(at.x - box.left + 14, box.width - 280)}px`;
          tip.style.top = `${at.y - box.top + 16}px`;
        },
        lost: () => {
          scene?.dispose();
          sceneRef.current = undefined;
          builtRef.current = undefined;
          setFailure('lost');
        },
      });
      sceneRef.current = scene;
      builtRef.current = specRef.current;
      scene.setSpec(specRef.current, 'overview');
      scene.setFocus(focusStateRef.current);
    } catch {
      scene?.dispose();
      sceneRef.current = undefined;
      builtRef.current = undefined;
      setFailure('unavailable');
    }
    return () => {
      scene?.dispose();
      sceneRef.current = undefined;
      builtRef.current = undefined;
    };
  }, []);

  // A live model change redraws the content and leaves the camera where the person put it.
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene || builtRef.current === spec) return;
    builtRef.current = spec;
    scene.setSpec(spec);
    setStep((s) => (s !== undefined && s >= spec.steps.length ? undefined : s));
  }, [spec]);

  useEffect(() => {
    sceneRef.current?.setFocus(focusState);
  }, [focusState]);

  useEffect(() => {
    sceneRef.current?.setStep(step);
  }, [step]);

  // A focus request (from the Inspector, the sidebar or a deep link) flies to
  // the entity. A click on the canvas only selects, as on the Board.
  const focusKey = focus && !focus.pending ? focus.key : undefined;
  const focusN = focus?.n;
  useEffect(() => {
    if (focusKey === undefined || focusN === undefined) return;
    const current = specRef.current;
    const pinned = selectionsRef.current.find(
      (s): s is Extract<typeof s, { type: 'entity' }> => s.type === 'entity' && s.key === focusKey && Boolean(s.occId),
    );
    const matches = current.nodes.filter((n) => n.entity.key === focusKey);
    const target = matches.find((n) => n.occId === pinned?.occId) ?? matches[0];
    if (target) sceneRef.current?.focusOn([target.id]);
  }, [focusKey, focusN]);

  const goStep = (delta: number) => {
    const n = spec.steps.length;
    if (n === 0) return;
    const next = step === undefined ? (delta > 0 ? 0 : n - 1) : Math.min(n - 1, Math.max(0, step + delta));
    setStep(next);
    sceneRef.current?.goTo({ step: next });
  };
  const preset = (name: 'front' | 'overview') => {
    setStep(undefined);
    sceneRef.current?.goTo(name);
  };

  if (failure) {
    return (
      <div className="relative h-full w-full">
        {failure === 'lost' ? (
          <BoardEmptyState icon={Box} title="The 3D view stopped">
            The browser took the graphics context back, for example after a graphics driver reset. Open another view and
            come back to draw it again.
          </BoardEmptyState>
        ) : (
          <BoardEmptyState icon={Box} title="This view needs WebGL">
            This browser could not start 3D graphics. The other views work without it.
          </BoardEmptyState>
        )}
      </div>
    );
  }

  const stepLabel = step === undefined ? undefined : spec.steps[step]?.label;
  return (
    <div className="relative h-full w-full overflow-hidden" style={{ background: BACKDROP }}>
      <div ref={hostRef} data-testid="graph-host" className="absolute inset-0" />
      <div
        ref={tipRef}
        hidden
        className="pointer-events-none absolute z-10 max-w-[16rem] rounded-md border border-border bg-card/95 px-2.5 py-1.5 text-xs shadow-md"
      >
        <div className="font-semibold" />
        <div className="text-[11px] text-muted-foreground" />
      </div>
      <BoardTopRight>
        <div className="flex items-center gap-0.5 rounded-lg border border-border bg-card/95 p-1 shadow-sm">
          <Button variant="ghost" size="sm" onClick={() => preset('front')}>
            Front
          </Button>
          <Button variant="ghost" size="sm" onClick={() => preset('overview')}>
            Overview
          </Button>
          {spec.steps.length > 0 ? (
            <>
              <Button
                variant="ghost"
                size="icon"
                aria-label={`Previous ${spec.stepNoun.toLowerCase()}`}
                onClick={() => goStep(-1)}
              >
                <ChevronLeft />
              </Button>
              <span
                className="max-w-[16rem] truncate px-1 text-xs tabular-nums text-muted-foreground"
                aria-live="polite"
              >
                {step === undefined
                  ? `Step through ${spec.steps.length} ${spec.stepNoun.toLowerCase()}s`
                  : `${spec.stepNoun} ${step + 1} of ${spec.steps.length}: ${stepLabel}`}
              </span>
              <Button
                variant="ghost"
                size="icon"
                aria-label={`Next ${spec.stepNoun.toLowerCase()}`}
                onClick={() => goStep(1)}
              >
                <ChevronRight />
              </Button>
            </>
          ) : null}
        </div>
      </BoardTopRight>
      <div className="pointer-events-none absolute bottom-3 right-3 max-w-xs rounded-md border border-border bg-card/95 px-2.5 py-2 text-[10px] text-muted-foreground">
        {spec.legend.map((line) => (
          <div key={line}>{line}</div>
        ))}
        <div>Drag to orbit, right-drag to pan, scroll to zoom.</div>
      </div>
      {spec.nodes.length === 0 ? (
        <BoardEmptyState icon={Box} title="Nothing to draw yet">
          This model has no commands, events or read models on its slices.
        </BoardEmptyState>
      ) : null}
    </div>
  );
}

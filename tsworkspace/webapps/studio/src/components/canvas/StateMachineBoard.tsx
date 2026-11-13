// The state machine view (see lib/statemachine.ts): one stream's lifecycle
// at a time, with time running left to right. The drawing fits the pane. A
// lifecycle too wide to fit at a readable size scrolls sideways, never
// down. Arrows and marks are one SVG layer; states are buttons placed over
// it. The guards of the selected state are listed under the drawing, so the
// arrows themselves carry no text.

import { CornerUpLeft, Workflow } from 'lucide-react';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { Selection } from '@/components/canvas/Board';
import { kindStyle } from '@/components/canvas/StickyNode';
import { IssueBadge } from '@/components/IssueBadge';
import { useIssues } from '@/components/IssuesProvider';
import { BoardEmptyState } from '@/components/ui/empty-state';
import { type Model, slugKey } from '@/lib/model';
import { arrowPath, buildLifecycle, type LifecycleState, type LifecycleStub, lifecycleLanes } from '@/lib/statemachine';
import { cn } from '@/lib/utils';
import type { FocusRequest } from './useFocusNode';

// Below this scale the labels get too small to read, so the drawing keeps
// its height and scrolls sideways instead of shrinking further.
const MIN_SCALE = 0.8;
const FIT_MARGIN = 16;
const RING = 'var(--ring)';
const EVENT = kindStyle('event');

const KIND_TEXT: Record<LifecycleState['kind'], string> = {
  row: 'the table lists what comes next',
  open: 'no row, so the table puts no limit on what comes next',
  final: 'final, nothing comes next',
};
const PROBLEM_TEXT: Record<NonNullable<LifecycleState['problem']>, string> = {
  missing: 'No event in this model has this id.',
  foreign: 'This event belongs to another stream. A lifecycle holds only its own stream’s events.',
};

function useBoxSize(ref: React.RefObject<HTMLDivElement | null>) {
  const [size, setSize] = useState<{ w: number; h: number }>();
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setSize({ w: entry.contentRect.width, h: entry.contentRect.height });
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return size;
}

function KeyItem({ children, mark }: { children: React.ReactNode; mark: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
      <svg aria-hidden width="26" height="14" className="overflow-visible">
        {mark}
      </svg>
      {children}
    </span>
  );
}

const place = (s: { x: number; y: number; w: number; h: number }) => ({
  left: s.x - s.w / 2,
  top: s.y - s.h / 2,
  width: s.w,
  height: s.h,
});

export function StateMachineBoard({
  model,
  selections,
  onSelect,
  focus,
  fitWhole = false,
}: {
  model: Model;
  selections: Selection[];
  onSelect: (key: string | undefined, occId?: string, additive?: boolean) => void;
  focus?: FocusRequest;
  /** A small frame, such as the Feature Preview, shows the whole drawing instead of a readable part of it. */
  fitWhole?: boolean;
}) {
  const lanes = useMemo(() => lifecycleLanes(model), [model]);
  const [laneKey, setLaneKey] = useState<string>();
  const lane = lanes.find((l) => slugKey('swimlane', l.id) === laneKey) ?? lanes[0];
  const lc = useMemo(() => (lane ? buildLifecycle(model, lane) : undefined), [model, lane]);
  const issues = useIssues();
  const markerId = useId().replace(/:/g, '');

  const selected = useMemo(
    () => new Set(selections.filter((s) => s.type === 'entity').map((s) => s.key)),
    [selections],
  );

  // A selection made elsewhere (sidebar, inspector) shows its stream. focus.n
  // is the intentional re-trigger, as in useFocusNode.
  const focusKey = focus && !focus.pending ? focus.key : undefined;
  const focusN = focus?.n;
  const latest = useRef({ model, lanes });
  latest.current = { model, lanes };
  useEffect(() => {
    if (focusKey === undefined || focusN === undefined) return;
    const { model: m, lanes: ls } = latest.current;
    const entity = m.byKey.get(focusKey);
    const home = entity?.kind === 'swimlane' ? entity.id : entity?.swimlane;
    if (home && ls.some((l) => slugKey('swimlane', l.id) === slugKey('swimlane', home)))
      setLaneKey(slugKey('swimlane', home));
  }, [focusKey, focusN]);

  const pane = useRef<HTMLDivElement>(null);
  const size = useBoxSize(pane);

  if (!lc || !lane) {
    return (
      <BoardEmptyState icon={Workflow} title="No lifecycle to draw">
        No stream in this model declares transitions yet.
      </BoardEmptyState>
    );
  }

  const byHeight = size ? Math.min(1, (size.h - FIT_MARGIN) / lc.height) : 1;
  const byWidth = size ? (size.w - FIT_MARGIN) / lc.width : 1;
  const scale = Math.max(0.1, fitWhole || byWidth >= MIN_SCALE ? Math.min(byHeight, byWidth) : byHeight);
  const width = Math.round(lc.width * scale);
  const height = Math.round(lc.height * scale);

  const stateById = new Map(lc.states.map((s) => [s.id, s]));
  const current = lc.states.find((s) => s.entity && selected.has(s.entity.key));
  const hot = new Set(
    current ? lc.arrows.filter((a) => a.from === current.id || a.target === current.id).map((a) => a.id) : [],
  );
  const near = new Set(lc.arrows.filter((a) => hot.has(a.id)).flatMap((a) => [a.from, a.to]));
  const next = current ? lc.arrows.filter((a) => a.from === current.id).sort((a, b) => a.order - b.order) : [];
  const returns = lc.arrows.filter((a) => a.reentry).length;
  const dim = (id: string) => Boolean(current) && current?.id !== id && !near.has(id);

  const choose = (e: React.MouseEvent, state: LifecycleState | undefined) => {
    if (state?.entity) onSelect(state.entity.key, undefined, e.metaKey || e.ctrlKey || e.shiftKey);
  };

  const renderState = (s: LifecycleState) => {
    const isCurrent = current?.id === s.id;
    return (
      <button
        key={s.id}
        type="button"
        disabled={!s.entity}
        aria-pressed={isCurrent}
        aria-label={`${s.title}: ${KIND_TEXT[s.kind]}${s.problem ? `. ${PROBLEM_TEXT[s.problem]}` : ''}`}
        title={s.problem ? `${s.ref.slug}. ${PROBLEM_TEXT[s.problem]}` : s.ref.slug}
        data-state={s.ref.slug}
        data-kind={s.kind}
        data-problem={s.problem}
        onClick={(e) => choose(e, s)}
        className={cn(
          'absolute flex items-center gap-1 rounded-[5px] border pr-3 pl-2.5 text-left outline-none',
          'focus-visible:ring-2 focus-visible:ring-ring enabled:cursor-pointer',
          s.problem === 'missing' ? 'border-amber-500 bg-card' : [EVENT.bg, EVENT.border],
          s.problem && 'border-dashed',
          isCurrent && 'border-2 border-ring',
          dim(s.id) && 'opacity-35',
        )}
        style={place(s)}
      >
        <span className="line-clamp-2 min-w-0 flex-1 text-[12px] leading-[15px] font-medium text-zinc-900">
          {s.title}
          {s.problem === 'missing' ? <span className="block text-[10px] text-amber-700">Missing event</span> : null}
        </span>
        <IssueBadge issues={s.entity ? issues.for(s.entity) : []} />
        {s.kind === 'final' ? (
          <span aria-hidden className="absolute top-[9px] right-[5px] bottom-[9px] w-1 rounded-sm bg-zinc-800" />
        ) : null}
      </button>
    );
  };

  const renderStub = (s: LifecycleStub) => {
    const target = stateById.get(s.returnsTo);
    const lit = current?.id === s.returnsTo || near.has(s.id);
    return (
      <button
        key={s.id}
        type="button"
        disabled={!target?.entity}
        aria-label={`Back to ${s.title}`}
        title={`Back to ${s.title}`}
        data-stub={s.id}
        onClick={(e) => choose(e, target)}
        className={cn(
          'absolute flex items-center gap-1.5 rounded-[14px] border border-dashed border-orange-400 bg-orange-50 px-3 text-left text-[12px] text-zinc-800 outline-none',
          'focus-visible:ring-2 focus-visible:ring-ring enabled:cursor-pointer',
          current?.id === s.returnsTo && 'border-2 border-ring',
          current && !lit && 'opacity-35',
        )}
        style={place(s)}
      >
        <CornerUpLeft aria-hidden className="h-3 w-3 shrink-0" />
        <span className="line-clamp-2 min-w-0 flex-1 leading-[15px]">{s.title}</span>
      </button>
    );
  };

  return (
    <div className="absolute inset-x-0 bottom-0 top-12 flex flex-col bg-background" data-testid="state-machine">
      <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1.5 px-3 pb-2 text-[11px]">
        {lanes.length > 1 ? (
          <fieldset aria-label="Stream" className="inline-flex h-7 items-center rounded-md bg-muted p-[3px]">
            {lanes.map((l) => {
              const k = slugKey('swimlane', l.id);
              const on = l === lane;
              return (
                <button
                  key={k}
                  type="button"
                  aria-pressed={on}
                  onClick={() => setLaneKey(k)}
                  className={cn(
                    'h-full whitespace-nowrap rounded px-2.5 text-xs font-medium text-muted-foreground hover:text-foreground',
                    on && 'bg-background text-foreground shadow-sm',
                  )}
                >
                  {l.title}
                </button>
              );
            })}
          </fieldset>
        ) : (
          <span className="text-xs font-semibold text-foreground">{lane.title}</span>
        )}
        <IssueBadge issues={issues.for(lane)} />
        <span className="font-semibold text-foreground">
          {`${lc.states.length} states · ${lc.arrows.length} transitions · ${returns} ${returns === 1 ? 'return' : 'returns'}`}
        </span>
        <span className="text-muted-foreground">
          Time runs left to right. A return to an earlier state ends in a stub.
        </span>
        <span className="ml-auto flex flex-wrap items-center gap-x-3 gap-y-1 text-muted-foreground">
          <KeyItem
            mark={
              <>
                <rect x="0.5" y="0.5" width="21" height="13" rx="3" className="fill-orange-200 stroke-orange-400" />
                <rect x="16" y="3" width="3" height="8" rx="1" className="fill-zinc-800" />
              </>
            }
          >
            Final
          </KeyItem>
          <KeyItem
            mark={
              <>
                <rect x="0.5" y="0.5" width="17" height="13" rx="3" className="fill-orange-200 stroke-orange-400" />
                <path d="M19 7 l6 -4 M19 7 h7 M19 7 l6 4" strokeLinecap="round" className="fill-none stroke-zinc-400" />
              </>
            }
          >
            No row, anything may follow
          </KeyItem>
          <KeyItem
            mark={
              <rect
                x="0.5"
                y="0.5"
                width="25"
                height="13"
                rx="4.5"
                strokeDasharray="3 2"
                className="fill-orange-50 stroke-orange-400"
              />
            }
          >
            Back to an earlier state
          </KeyItem>
        </span>
      </div>
      <div ref={pane} className="flex min-h-0 flex-1 overflow-x-auto overflow-y-hidden">
        <section aria-label={`${lane.title} lifecycle`} className="relative m-auto shrink-0" style={{ width, height }}>
          <svg
            aria-hidden
            width={width}
            height={height}
            viewBox={`0 0 ${lc.width} ${lc.height}`}
            className="absolute inset-0"
          >
            <defs>
              {(['', '-hot'] as const).map((suffix) => (
                <marker
                  key={suffix}
                  id={`${markerId}${suffix}`}
                  viewBox="0 0 8 8"
                  refX={1}
                  refY={4}
                  markerWidth={7}
                  markerHeight={7}
                  orient="auto"
                >
                  <path
                    d="M0,0 L8,4 L0,8 z"
                    className={suffix ? undefined : 'fill-zinc-400'}
                    style={suffix ? { fill: RING } : undefined}
                  />
                </marker>
              ))}
            </defs>
            {lc.arrows.map((a) => {
              const isHot = hot.has(a.id);
              return (
                <path
                  key={a.id}
                  d={arrowPath(a.points)}
                  data-arrow={a.id}
                  fill="none"
                  strokeWidth={isHot ? 2 : 1.3}
                  strokeDasharray={a.reentry ? '4 3' : undefined}
                  markerEnd={`url(#${markerId}${isHot ? '-hot' : ''})`}
                  className={cn(!isHot && 'stroke-zinc-400', current && !isHot && 'opacity-20')}
                  style={isHot ? { stroke: RING } : undefined}
                />
              );
            })}
            {lc.states
              .filter((s) => s.kind === 'open')
              .map((s) => {
                const x = s.x + s.w / 2 + 4;
                return (
                  <path
                    key={s.id}
                    d={`M${x},${s.y} l12,-8 M${x},${s.y} h14 M${x},${s.y} l12,8`}
                    strokeWidth={1.25}
                    strokeLinecap="round"
                    className={cn('fill-none stroke-zinc-400', dim(s.id) && 'opacity-35')}
                  />
                );
              })}
            {next.map((a, i) => {
              const end = a.points[a.points.length - 1];
              return (
                <g key={a.id} transform={`translate(${end.x - 10},${end.y})`}>
                  <circle r={8} style={{ fill: RING }} />
                  <text textAnchor="middle" dominantBaseline="central" className="fill-white text-[10px] font-semibold">
                    {i + 1}
                  </text>
                </g>
              );
            })}
          </svg>
          <div
            className="absolute top-0 left-0 origin-top-left"
            style={{ width: lc.width, height: lc.height, transform: `scale(${scale})` }}
          >
            {lc.states.map(renderState)}
            {lc.stubs.map(renderStub)}
          </div>
        </section>
      </div>
      {current ? (
        <div className="shrink-0 border-t border-border bg-card px-3 py-2 text-[12px]" data-testid="state-machine-next">
          <div className="font-semibold text-foreground">
            After {current.title}: {KIND_TEXT[current.kind]}
          </div>
          {next.length ? (
            <ol className="mt-1.5 grid gap-x-6 gap-y-1 md:grid-cols-2">
              {next.map((a, i) => (
                <li key={a.id} className="flex min-w-0 gap-2">
                  <span
                    className="mt-px grid h-4 w-4 shrink-0 place-items-center rounded-full text-[10px] font-semibold text-white"
                    style={{ background: RING }}
                  >
                    {i + 1}
                  </span>
                  <span className="min-w-0">
                    <span className="font-medium text-foreground">
                      {stateById.get(a.target)?.title ?? a.target}
                      {a.reentry ? ' (back to an earlier state)' : ''}
                    </span>{' '}
                    <span className="text-muted-foreground">{a.guard || 'No guard written.'}</span>
                  </span>
                </li>
              ))}
            </ol>
          ) : null}
        </div>
      ) : null}
      {lc.untracked.length ? (
        <div
          className="shrink-0 truncate px-3 pb-2 text-[11px] text-muted-foreground"
          title={lc.untracked.map((e) => e.title).join('\n')}
        >
          {lc.untracked.length === 1
            ? '1 event on this stream is in no transition: '
            : `${lc.untracked.length} events on this stream are in no transition: `}
          {lc.untracked.map((e) => e.title).join(', ')}
        </div>
      ) : null}
    </div>
  );
}

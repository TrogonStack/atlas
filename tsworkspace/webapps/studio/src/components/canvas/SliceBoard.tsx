// The Slice view: one slice, the step that feeds it and the step it feeds,
// with its fields, its validator findings, its story and its scenarios.
// Read only, like every Studio view: a card click selects the entity so the
// Inspector opens, and nothing here changes the model.
import { ChevronLeft, ChevronRight, Columns3, Pause, Play, X } from 'lucide-react';
import { useQueryState } from 'nuqs';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Selection } from '@/components/canvas/Board';
import { BoardTopRight } from '@/components/canvas/BoardTopRight';
import { GlyphIcon } from '@/components/canvas/GlyphIcon';
import { kindStyle, StickyCardBody } from '@/components/canvas/StickyNode';
import type { FocusRequest } from '@/components/canvas/useFocusNode';
import { ExampleBlock } from '@/components/ExampleBlock';
import { IssueBadge } from '@/components/IssueBadge';
import { IssueList } from '@/components/IssueList';
import { useIssues } from '@/components/IssuesProvider';
import { Button } from '@/components/ui/button';
import { BoardEmptyState } from '@/components/ui/empty-state';
import { Tabs, TabsList, TabsPanel, TabsTab } from '@/components/ui/tabs';
import type { Severity } from '@/lib/issues';
import {
  type Entity,
  entityKey,
  fieldTypeLabel,
  instanceUidOf,
  type Model,
  orderedSlices,
  type ScenarioView,
  slugKey,
  trackingOf,
} from '@/lib/model';
import {
  boardFieldNames,
  boardOrder,
  cardSlugKey,
  type FieldFlow,
  type ScenarioPhase,
  fieldFlow,
  fieldOf,
  isFieldCarrier,
  SLICE_CARD_H,
  SLICE_CARD_W,
  type SliceCard,
  type SliceNeighborhood,
  scenarioStepKeys,
  scenarioSteps,
  sliceBoardGeometry,
  sliceByParam,
  sliceForEntity,
  sliceGroups,
  sliceMembers,
  sliceNeighborhood,
  sliceParam,
  storyboardsOf,
  storySteps,
  streamStateField,
} from '@/lib/sliceview';
import { cn } from '@/lib/utils';

/** How long playback stays on one step. */
export const PLAY_STEP_MS = 1600;

const PILL_STYLE: Record<Severity, string> = {
  error: 'bg-red-100 text-red-800',
  warning: 'bg-amber-100 text-amber-900',
  info: 'bg-sky-100 text-sky-800',
};

type PanelTab = 'fields' | 'validation' | 'story' | 'scenarios';

/** One highlight for the board at a time: a traced field, or the entities a scenario step names. */
type Highlight = { kind: 'field'; flow: FieldFlow } | { kind: 'scenario'; keys: Set<string> } | undefined;

/**
 * Steps through `length` positions. Play advances one step per interval and
 * stops on the last one; Play on the last step starts again from the first.
 */
function usePlayback(length: number, index: number, go: (index: number) => void) {
  const [playing, setPlaying] = useState(false);
  const goRef = useRef(go);
  goRef.current = go;
  // Play-from-end calls go(0) before the parent has switched slice/index. Hold
  // playing until that lands so the "stop on last" branch does not cancel it.
  const awaitingRestart = useRef(false);
  useEffect(() => {
    if (!playing) return;
    if (index < 0 || index >= length - 1) {
      if (awaitingRestart.current && length > 1 && index === length - 1) return;
      awaitingRestart.current = false;
      setPlaying(false);
      return;
    }
    awaitingRestart.current = false;
    const t = window.setTimeout(() => goRef.current(index + 1), PLAY_STEP_MS);
    return () => window.clearTimeout(t);
  }, [playing, index, length]);
  const stop = useCallback(() => {
    awaitingRestart.current = false;
    setPlaying(false);
  }, []);
  const toggle = () => {
    if (playing) {
      awaitingRestart.current = false;
      setPlaying(false);
      return;
    }
    if (index >= length - 1) {
      awaitingRestart.current = length > 1;
      goRef.current(0);
    }
    setPlaying(true);
  };
  return { playing, toggle, stop };
}

function PlaybackControls({
  index,
  length,
  playing,
  onPrevious,
  onNext,
  onToggle,
  label,
}: {
  index: number;
  length: number;
  playing: boolean;
  onPrevious: () => void;
  onNext: () => void;
  onToggle: () => void;
  label: string;
}) {
  return (
    <div className="flex items-center gap-1.5">
      <Button variant="outline" size="sm" onClick={onPrevious} disabled={index <= 0} aria-label={`Previous ${label}`}>
        <ChevronLeft />
        Previous
      </Button>
      <Button
        variant="outline"
        size="sm"
        onClick={onToggle}
        disabled={length < 2 || index < 0}
        aria-label={playing ? 'Pause' : 'Play'}
      >
        {playing ? <Pause /> : <Play />}
        {playing ? 'Pause' : 'Play'}
      </Button>
      <Button
        variant="outline"
        size="sm"
        onClick={onNext}
        disabled={index < 0 || index >= length - 1}
        aria-label={`Next ${label}`}
      >
        Next
        <ChevronRight />
      </Button>
      <span className="ml-1 text-[11px] text-muted-foreground">
        {index >= 0 ? `${index + 1} of ${length}` : `not in this story`}
      </span>
    </div>
  );
}

// What a card says about the traced field, under its title.
function TraceLine({ model, card, flow }: { model: Model; card: SliceCard; flow: FieldFlow }) {
  const presence = flow.presence.get(card.id);
  const name = flow.name;
  if (presence === 'carries') {
    const type = fieldTypeLabel(fieldOf(card.entity, name) ?? {});
    return (
      <span className="font-semibold text-zinc-800">
        {name}
        {type ? <span className="font-normal text-zinc-500"> · {type}</span> : null}
      </span>
    );
  }
  if (presence === 'state') {
    const type = fieldTypeLabel(streamStateField(model, card.entity, name) ?? {});
    return (
      <span className="font-semibold text-zinc-800">
        {name}
        <span className="font-normal text-zinc-500"> · stream state{type ? `, ${type}` : ''}</span>
      </span>
    );
  }
  if (presence === 'passes') return <span className="text-zinc-600">passes {name} on</span>;
  if (presence === 'lacks') return <span className="text-zinc-500">no {name}</span>;
  return null;
}

function BoardCard({
  model,
  card,
  x,
  y,
  selected,
  emphasis,
  flow,
  onSelect,
}: {
  model: Model;
  card: SliceCard;
  x: number;
  y: number;
  selected: boolean;
  emphasis?: 'lit' | 'dim';
  flow?: FieldFlow;
  onSelect: (card: SliceCard, additive: boolean) => void;
}) {
  const style = kindStyle(card.kind);
  const position = { left: x, top: y, width: SLICE_CARD_W, height: SLICE_CARD_H };
  const trace = flow ? <TraceLine model={model} card={card} flow={flow} /> : null;
  const entity = card.entity;
  if (!entity) {
    // The slice names an entity this model does not load: keep the gap
    // visible instead of drawing the slice as if it were complete.
    return (
      <div
        data-card={card.id}
        data-emphasis={emphasis}
        title={`${card.ref.namespace}/${card.ref.slug}@${card.ref.version} is not loaded`}
        style={position}
        className={cn(
          'absolute flex flex-col overflow-hidden rounded-md border-2 border-dashed border-zinc-300 bg-zinc-50 px-3 py-2 transition-opacity',
          emphasis === 'dim' && 'opacity-30',
        )}
      >
        <div className="text-[10px] font-semibold uppercase tracking-wide text-zinc-500">{style.label}</div>
        <div className="mt-1 line-clamp-2 text-[12px] font-semibold text-zinc-600 wrap-anywhere">{card.ref.slug}</div>
        <div className="mt-auto text-[10px] italic text-zinc-500">not loaded</div>
      </div>
    );
  }
  return (
    <button
      type="button"
      data-card={card.id}
      data-entity-key={entity.key}
      data-emphasis={emphasis}
      aria-label={`${style.label}: ${entity.title}`}
      aria-pressed={selected}
      onClick={(e) => {
        e.stopPropagation();
        onSelect(card, e.metaKey || e.ctrlKey || e.shiftKey);
      }}
      style={position}
      className={cn(
        'absolute flex flex-col overflow-hidden rounded-md border-2 px-3 py-2 text-left shadow-[2px_3px_0_rgba(0,0,0,0.08)] transition-opacity',
        style.bg,
        style.border,
        emphasis === 'lit' && 'ring-2 ring-zinc-700 ring-offset-2',
        emphasis === 'dim' && 'opacity-30',
        selected && 'ring-2 ring-ring ring-offset-0 shadow-[3px_5px_0_rgba(0,0,0,0.14)]',
      )}
    >
      <StickyCardBody entity={entity} />
      {trace ? <div className="mt-auto truncate font-mono text-[10px]">{trace}</div> : null}
    </button>
  );
}

function SliceBoardCanvas({
  model,
  hood,
  selectedKeys,
  highlight,
  onSelect,
  onSelectCard,
}: {
  model: Model;
  hood: SliceNeighborhood;
  selectedKeys: Set<string>;
  highlight: Highlight;
  onSelect: (key: string | undefined, additive?: boolean) => void;
  onSelectCard: (card: SliceCard, additive: boolean) => void;
}) {
  const geo = useMemo(() => sliceBoardGeometry(hood), [hood]);
  const slice = hood.slice;
  const issues = useIssues().for(slice.entity);
  const status = trackingOf(model, slice.entity)[0]?.status;
  const headerStyle = kindStyle(slice.kind);
  const own = geo.steps.filter((s) => s.step.role === 'slice');
  const ownStart = own[0]?.x ?? 0;
  const ownEnd = (own[own.length - 1]?.x ?? 0) + SLICE_CARD_W;
  const flow = highlight?.kind === 'field' ? highlight.flow : undefined;
  const emphasisOf = (card: SliceCard): 'lit' | 'dim' | undefined => {
    if (!highlight) return undefined;
    if (highlight.kind === 'scenario') return highlight.keys.has(cardSlugKey(card)) ? 'lit' : 'dim';
    const p = highlight.flow.presence.get(card.id);
    return p === 'carries' || p === 'state' || p === 'passes' ? 'lit' : 'dim';
  };
  const lit = new Set(hood.cards.filter((c) => emphasisOf(c) === 'lit').map((c) => c.id));
  const titleOf = cardTitleLookup(hood);
  return (
    // The surface itself is the pane: a click on empty space clears the
    // selection, as on the other boards. Esc does the same from the keyboard.
    // oxlint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
    <div
      data-testid="slice-canvas"
      className="relative"
      style={{ width: geo.width, height: geo.height }}
      onClick={() => onSelect(undefined)}
    >
      {geo.lanes.map(({ lane, y, height }) => (
        <div
          key={lane.id}
          data-lane={lane.id}
          className="absolute rounded-lg border border-dashed border-zinc-300/80"
          style={{ left: 8, top: y, width: geo.width - 16, height, background: lane.tint }}
        >
          <div className="flex w-[104px] items-start gap-1.5 px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">
            <GlyphIcon glyph={lane.glyph} className="mt-px h-3 w-3 shrink-0" />
            {lane.entity ? (
              <button
                type="button"
                className="min-w-0 text-left uppercase hover:text-zinc-800"
                onClick={(e) => {
                  e.stopPropagation();
                  if (lane.entity) onSelect(lane.entity.key);
                }}
              >
                {lane.label}
              </button>
            ) : (
              <span className="min-w-0">{lane.label}</span>
            )}
          </div>
        </div>
      ))}
      {geo.steps
        .filter((s) => s.step.role !== 'slice')
        .map(({ step, x }) => (
          <div
            key={step.index}
            className="absolute flex items-end pb-1 text-[10px] font-semibold uppercase tracking-wide text-zinc-500"
            style={{ left: x, top: 4, width: SLICE_CARD_W, height: 36 }}
            title={step.role === 'upstream' ? 'What feeds this slice' : 'What this slice feeds'}
          >
            {step.role === 'upstream' ? 'Upstream' : 'Downstream'}
          </div>
        ))}
      {own.length > 0 ? (
        <button
          type="button"
          aria-label={`Slice: ${slice.entity.title}`}
          aria-pressed={selectedKeys.has(slice.entity.key)}
          onClick={(e) => {
            e.stopPropagation();
            onSelect(slice.entity.key, e.metaKey || e.ctrlKey || e.shiftKey);
          }}
          className={cn(
            'absolute rounded-md border px-2.5 py-1 text-left',
            headerStyle.bg,
            headerStyle.border,
            selectedKeys.has(slice.entity.key) && 'ring-2 ring-ring',
          )}
          style={{ left: ownStart, top: 4, width: ownEnd - ownStart, height: 36 }}
        >
          <div className="flex items-center gap-1 text-[9px] font-semibold uppercase tracking-wide text-zinc-500">
            {kindStyle(slice.entity.kind).label}
            <span className="ml-auto flex items-center gap-1">
              <IssueBadge issues={issues} />
              {slice.scenarios.length > 0 ? (
                <span className="rounded bg-white/70 px-1 text-[9px] normal-case">
                  {slice.scenarios.length === 1 ? '1 scenario' : `${slice.scenarios.length} scenarios`}
                </span>
              ) : null}
              {status ? <span className="rounded bg-white/70 px-1 font-mono text-[8px]">{status}</span> : null}
            </span>
          </div>
          <div className="truncate text-[12px] font-semibold leading-snug text-zinc-800">{slice.entity.title}</div>
        </button>
      ) : null}
      <svg
        aria-hidden="true"
        className="pointer-events-none absolute inset-0"
        width={geo.width}
        height={geo.height}
        viewBox={`0 0 ${geo.width} ${geo.height}`}
      >
        <defs>
          <marker id="slice-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto">
            <path d="M0,0 L10,5 L0,10 z" fill="#a1a1aa" />
          </marker>
          <marker
            id="slice-arrow-hot"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="6"
            markerHeight="6"
            orient="auto"
          >
            <path d="M0,0 L10,5 L0,10 z" fill="#3f3f46" />
          </marker>
        </defs>
        {hood.links.map((l) => {
          const path = geo.links.get(l.id);
          if (!path) return null;
          const hot = flow ? flow.carried.has(l.id) : highlight ? lit.has(l.source) && lit.has(l.target) : false;
          const dim = highlight && !hot;
          return (
            <path
              key={l.id}
              data-link={l.id}
              data-hot={hot || undefined}
              d={path.d}
              fill="none"
              stroke={hot ? '#3f3f46' : '#a1a1aa'}
              strokeWidth={hot ? 2.4 : 1.6}
              opacity={dim ? 0.25 : 1}
              markerEnd={hot ? 'url(#slice-arrow-hot)' : 'url(#slice-arrow)'}
            />
          );
        })}
        {flow
          ? hood.links
              .filter((l) => flow.stops.has(l.id))
              .map((l) => {
                const mid = geo.links.get(l.id)?.mid;
                if (!mid) return null;
                return (
                  <g key={l.id} data-testid="field-stop" transform={`translate(${mid.x},${mid.y})`}>
                    <title>{`${flow.name} stops here: ${titleOf(l.source)} has it, ${titleOf(l.target)} does not`}</title>
                    <circle r="9" fill="#3f3f46" stroke="#ffffff" strokeWidth="2" />
                    <path d="M-3.5,-3.5 L3.5,3.5 M3.5,-3.5 L-3.5,3.5" stroke="#ffffff" strokeWidth="2" />
                  </g>
                );
              })
          : null}
      </svg>
      {hood.cards.map((card) => {
        const p = geo.cards.get(card.id);
        if (!p) return null;
        return (
          <BoardCard
            key={card.id}
            model={model}
            card={card}
            x={p.x}
            y={p.y}
            selected={Boolean(card.entity && selectedKeys.has(card.entity.key))}
            emphasis={emphasisOf(card)}
            flow={flow}
            onSelect={onSelectCard}
          />
        );
      })}
    </div>
  );
}

function EntityChip({ entity, onSelect }: { entity: Entity; onSelect: (key: string) => void }) {
  const style = kindStyle(entity.kind);
  const Icon = style.icon;
  return (
    <button
      type="button"
      onClick={() => onSelect(entity.key)}
      className="inline-flex min-w-0 items-center gap-1.5 rounded-md border border-border px-1.5 py-0.5 text-[11px] hover:bg-accent"
    >
      <Icon className="h-3 w-3 shrink-0 text-muted-foreground" />
      <span className="truncate font-medium">{entity.title}</span>
      <span className="shrink-0 text-[9px] uppercase tracking-wide text-muted-foreground">{style.label}</span>
    </button>
  );
}

function FieldsPanel({
  hood,
  model,
  traced,
  flow,
  onTrace,
  onSelect,
}: {
  hood: SliceNeighborhood;
  model: Model;
  traced?: string;
  flow?: FieldFlow;
  onTrace: (name: string | undefined) => void;
  onSelect: (key: string) => void;
}) {
  const names = useMemo(() => boardFieldNames(hood), [hood]);
  const columns = useMemo(() => boardOrder(hood).filter((c) => isFieldCarrier(c.kind) && c.entity), [hood]);
  const titleOf = cardTitleLookup(hood);
  if (names.length === 0 && !traced)
    return <p className="text-[11px] italic text-muted-foreground">No card on this board declares a schema field.</p>;
  const holders = flow ? hood.cards.filter((c) => ['carries', 'state'].includes(flow.presence.get(c.id) ?? '')) : [];
  const stops = flow ? hood.links.filter((l) => flow.stops.has(l.id)) : [];
  return (
    <div className="space-y-2">
      {flow ? (
        <div className="flex flex-wrap items-center gap-2 rounded-md border border-border bg-muted/40 px-2 py-1.5 text-[11px]">
          <span>
            <span className="font-mono font-semibold">{flow.name}</span>{' '}
            {holders.length === 0
              ? 'is on no card of this board.'
              : `is on ${holders.length} of ${hood.cards.length} cards.`}{' '}
            {stops.length === 0
              ? 'It does not stop at any link here.'
              : `It stops at ${stops.length === 1 ? '1 link' : `${stops.length} links`}: ${stops
                  .map((l) => `${titleOf(l.source)} to ${titleOf(l.target)}`)
                  .join('; ')}.`}
          </span>
          <Button variant="ghost" size="sm" className="ml-auto h-6 px-2" onClick={() => onTrace(undefined)}>
            <X />
            Clear
          </Button>
        </div>
      ) : (
        <p className="text-[11px] text-muted-foreground">
          Pick a field to follow it along the board. Names match only along the links drawn here.
        </p>
      )}
      <div className="overflow-x-auto">
        <table className="border-separate border-spacing-0 text-[11px]">
          <thead>
            <tr>
              <th className="sticky left-0 bg-card px-2 py-1 text-left text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                Field
              </th>
              {columns.map((c) => {
                const style = kindStyle(c.kind);
                const entity = c.entity;
                if (!entity) return null;
                return (
                  <th key={c.id} className="px-1 py-1 align-bottom font-normal">
                    <button
                      type="button"
                      title={`${c.role === 'slice' ? 'this slice' : c.role}: ${entity.title}`}
                      onClick={() => onSelect(entity.key)}
                      className={cn(
                        'block w-28 truncate rounded border px-1.5 py-0.5 text-left text-[10px] font-semibold text-zinc-800',
                        style.bg,
                        style.border,
                        c.role !== 'slice' && 'opacity-70',
                      )}
                    >
                      {entity.title}
                    </button>
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {names.map((name) => (
              <tr key={name} className={cn(traced === name && 'bg-accent')}>
                <td className="sticky left-0 bg-inherit px-2 py-0.5">
                  <button
                    type="button"
                    aria-pressed={traced === name}
                    onClick={() => onTrace(traced === name ? undefined : name)}
                    className="font-mono text-[11px] hover:underline"
                  >
                    {name}
                  </button>
                </td>
                {columns.map((c) => {
                  const own = fieldOf(c.entity, name);
                  const state = own ? undefined : streamStateField(model, c.entity, name);
                  const style = kindStyle(c.kind);
                  return (
                    <td key={c.id} className="px-1 py-0.5 text-center">
                      {own ? (
                        <span
                          title={`${c.entity?.title}: ${name}${fieldTypeLabel(own) ? `, ${fieldTypeLabel(own)}` : ''}`}
                          className={cn('inline-block h-2.5 w-2.5 rounded-full border', style.bg, style.border)}
                        />
                      ) : state ? (
                        <span
                          title={`${c.entity?.title}: ${name} is kept as stream state`}
                          className="inline-block h-2.5 w-2.5 rounded-full border-2 border-dashed border-orange-400"
                        />
                      ) : null}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="flex items-center gap-3 text-[10px] text-muted-foreground">
        <span className="flex items-center gap-1">
          <span className="inline-block h-2.5 w-2.5 rounded-full border border-zinc-400 bg-zinc-200" />
          the card declares the field
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-2.5 w-2.5 rounded-full border-2 border-dashed border-orange-400" />
          the event's swimlane keeps it as state
        </span>
      </p>
    </div>
  );
}

function cardTitleLookup(hood: SliceNeighborhood): (cardId: string) => string {
  const titles = new Map(hood.cards.map((c) => [c.id, c.entity?.title ?? c.ref.slug]));
  return (cardId) => titles.get(cardId) ?? '';
}

function worstSeverity(issues: { severity: Severity }[]): Severity | undefined {
  return issues.reduce<Severity | undefined>(
    (acc, i) => (acc === 'error' || i.severity === 'error' ? 'error' : acc === 'warning' ? acc : i.severity),
    undefined,
  );
}

// The badge and the panel both read this list, so their counts cannot drift.
function validationEntities(model: Model, hood: SliceNeighborhood): Entity[] {
  const slice = hood.slice;
  const entities: Entity[] = [slice.entity];
  const seen = new Set([slice.entity.key]);
  for (const m of sliceMembers(slice)) {
    const entity = hood.cards.find((c) => c.role === 'slice' && cardSlugKey(c) === slugKey(m.kind, m.id))?.entity;
    const resolved = entity ?? model.entities.find((e) => e.key === entityKey(m.kind, m.id));
    if (resolved && !seen.has(resolved.key)) {
      seen.add(resolved.key);
      entities.push(resolved);
    }
  }
  return entities;
}

function ValidationPanel({
  model,
  hood,
  onSelect,
}: {
  model: Model;
  hood: SliceNeighborhood;
  onSelect: (key: string) => void;
}) {
  const issues = useIssues();
  const slice = hood.slice;
  const groups = useMemo(() => {
    const entities = validationEntities(model, hood);
    return entities.map((entity) => ({ entity, issues: issues.for(entity) })).filter((g) => g.issues.length > 0);
  }, [model, hood, issues]);
  if (issues.error)
    return (
      <p className="text-[11px] italic text-muted-foreground">
        Validation unavailable ({issues.error}). This view cannot say whether the slice is valid.
      </p>
    );
  if (groups.length === 0)
    return (
      <p className="text-[11px] italic text-muted-foreground">
        No validation findings for this slice or the entities it names.
      </p>
    );
  return (
    <div className="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(18rem,1fr))]">
      {groups.map((g) => (
        <section key={g.entity.key} aria-label={`Findings for ${g.entity.title}`} className="space-y-1.5">
          <div className="flex items-center gap-1.5">
            <EntityChip entity={g.entity} onSelect={onSelect} />
            <span
              className={cn(
                'rounded-full px-1.5 text-[10px] font-semibold',
                PILL_STYLE[worstSeverity(g.issues) ?? 'info'],
              )}
            >
              {g.issues.length}
            </span>
          </div>
          <IssueList issues={g.issues} model={model} momentKey={slice.entity.key} />
        </section>
      ))}
    </div>
  );
}

function StoryPanel({
  model,
  hood,
  storyboardKey,
  onStoryboard,
  position,
  onStep,
  picks,
}: {
  model: Model;
  hood: SliceNeighborhood;
  storyboardKey?: string;
  onStoryboard: (key: string) => void;
  position: number;
  onStep: (index: number) => void;
  /** Counts picker choices, so a manual pick ends playback that would otherwise carry on from the new slice. */
  picks: number;
}) {
  const slice = hood.slice;
  const storyboards = useMemo(() => storyboardsOf(model, slice), [model, slice]);
  const storyboard = storyboards.find((s) => s.key === storyboardKey) ?? storyboards[0];
  const steps = useMemo(() => (storyboard ? storySteps(model, storyboard) : undefined), [model, storyboard]);
  const index =
    steps && steps.slices[position]?.entity.key === slice.entity.key
      ? position
      : (steps?.slices.findIndex((s) => s.entity.key === slice.entity.key) ?? -1);
  const playback = usePlayback(steps?.slices.length ?? 0, index, onStep);
  const stopPlayback = playback.stop;
  useEffect(() => {
    stopPlayback();
  }, [picks, stopPlayback]);
  if (!storyboard || !steps)
    return (
      <p className="text-[11px] italic text-muted-foreground">
        No storyboard lists this slice, so there is no story to play.
      </p>
    );
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        {storyboards.length > 1 ? (
          <select
            aria-label="Storyboard"
            value={storyboard.key}
            onChange={(e) => {
              playback.stop();
              onStoryboard(e.target.value);
            }}
            className="rounded-md border border-border bg-card px-1.5 py-1 text-[11px]"
          >
            {storyboards.map((s) => (
              <option key={s.key} value={s.key}>
                {s.title}
              </option>
            ))}
          </select>
        ) : (
          <span className="text-[11px] font-semibold">{storyboard.title}</span>
        )}
        <PlaybackControls
          label="slice"
          index={index}
          length={steps.slices.length}
          playing={playback.playing}
          onToggle={playback.toggle}
          onPrevious={() => {
            playback.stop();
            onStep(index - 1);
          }}
          onNext={() => {
            playback.stop();
            onStep(index + 1);
          }}
        />
      </div>
      <ol aria-label={`${storyboard.title} slices`} className="flex flex-wrap gap-1.5">
        {steps.slices.map((s, i) => {
          const current = i === index;
          const header = kindStyle(s.kind);
          return (
            // oxlint-disable-next-line react/no-array-index-key -- a storyboard may list one slice twice
            <li key={`${i}:${s.entity.key}`}>
              <button
                type="button"
                aria-current={current ? 'step' : undefined}
                onClick={() => {
                  playback.stop();
                  onStep(i);
                }}
                className={cn(
                  'flex max-w-[15rem] items-center gap-1.5 rounded-md border px-2 py-1 text-left text-[11px]',
                  header.bg,
                  header.border,
                  current ? 'ring-2 ring-ring' : 'opacity-80 hover:opacity-100',
                )}
              >
                <span className="font-mono text-[10px] text-zinc-500">{i + 1}</span>
                <span className="truncate font-medium text-zinc-800">{s.entity.title}</span>
              </button>
            </li>
          );
        })}
      </ol>
      {steps.missing.length > 0 ? (
        <p className="text-[10px] text-muted-foreground">
          Not loaded, so playback skips {steps.missing.length === 1 ? 'it' : 'them'}:{' '}
          <span className="font-mono">{steps.missing.map((m) => `${m.namespace}/${m.slug}`).join(', ')}</span>
        </p>
      ) : null}
    </div>
  );
}

function scenarioOutcome(s: ScenarioView): string {
  if (s.reject) return `rejects ${s.reject.reasonCode}`;
  if (s.thenEmits.length > 0) return `emits ${s.thenEmits.map((e) => e.id.slug).join(', ')}`;
  if (s.thenState) return `state ${s.thenState.id.slug}`;
  if (s.thenCommand) return `issues ${s.thenCommand.id.slug}`;
  return '';
}

function phaseLabel(phase: ScenarioPhase): string {
  return phase === 'given' ? 'Given' : phase === 'when' ? 'When' : 'Then';
}

function ScenariosPanel({
  hood,
  scenario,
  step,
  onScenario,
  onStep,
}: {
  hood: SliceNeighborhood;
  scenario?: number;
  step: number;
  onScenario: (index: number | undefined) => void;
  onStep: (index: number) => void;
}) {
  const slice = hood.slice;
  const active = scenario !== undefined ? slice.scenarios[scenario] : undefined;
  const steps = useMemo(() => (active ? scenarioSteps(slice, active) : []), [slice, active]);
  const playback = usePlayback(steps.length, active ? step : -1, onStep);
  const onBoard = useMemo(() => new Set(hood.cards.map(cardSlugKey)), [hood]);
  if (slice.scenarios.length === 0)
    return <p className="text-[11px] italic text-muted-foreground">This slice has no scenarios.</p>;
  const current = steps[step];
  return (
    <div className="flex flex-col gap-3 md:flex-row">
      <ul aria-label="Scenarios" className="flex shrink-0 flex-col gap-1 md:w-60">
        {slice.scenarios.map((s, i) => (
          <li key={s.id || i}>
            <button
              type="button"
              aria-pressed={scenario === i}
              onClick={() => {
                playback.stop();
                onScenario(scenario === i ? undefined : i);
              }}
              className={cn(
                'w-full rounded-md border border-border px-2 py-1 text-left text-[11px] hover:bg-accent',
                scenario === i && 'border-ring bg-accent',
              )}
            >
              <span className="block truncate font-medium">{s.title || s.id}</span>
              <span
                className={cn(
                  'block truncate font-mono text-[10px]',
                  s.reject ? 'text-red-600' : 'text-muted-foreground',
                )}
              >
                {scenarioOutcome(s)}
              </span>
            </button>
          </li>
        ))}
      </ul>
      {active && current ? (
        <div className="min-w-0 flex-1 space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <ol aria-label="Scenario steps" className="flex items-center gap-1">
              {steps.map((s, i) => (
                <li key={s.phase}>
                  <button
                    type="button"
                    aria-current={i === step ? 'step' : undefined}
                    onClick={() => {
                      playback.stop();
                      onStep(i);
                    }}
                    className={cn(
                      'rounded-md border px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide',
                      i === step ? 'border-zinc-700 bg-zinc-800 text-white' : 'border-border text-muted-foreground',
                    )}
                  >
                    {phaseLabel(s.phase)}
                  </button>
                </li>
              ))}
            </ol>
            <PlaybackControls
              label="step"
              index={step}
              length={steps.length}
              playing={playback.playing}
              onToggle={playback.toggle}
              onPrevious={() => {
                playback.stop();
                onStep(step - 1);
              }}
              onNext={() => {
                playback.stop();
                onStep(step + 1);
              }}
            />
          </div>
          <section aria-label={phaseLabel(current.phase)} className="space-y-1.5">
            {current.doc ? <p className="text-[11px] italic">{current.doc}</p> : null}
            {current.examples.length === 0 && !current.doc && !current.reject ? (
              <p className="text-[11px] italic text-muted-foreground">nothing</p>
            ) : null}
            <div className="grid gap-1.5 [grid-template-columns:repeat(auto-fill,minmax(14rem,1fr))]">
              {current.examples.map((e, i) => (
                // oxlint-disable-next-line react/no-array-index-key -- a step may name one entity twice
                <div key={`${slugKey(e.kind, e.id)}:${i}`}>
                  <ExampleBlock label={kindStyle(e.kind).label} example={e} />
                  {onBoard.has(slugKey(e.kind, e.id)) ? null : (
                    <p className="mt-0.5 text-[10px] text-muted-foreground">not on this board</p>
                  )}
                </div>
              ))}
            </div>
            {current.reject ? (
              <div className="rounded-md border border-red-200 bg-red-50 px-2 py-1.5 text-[11px]">
                <span className="font-mono font-semibold text-red-700">{current.reject.reasonCode}</span>
                {current.reject.doc ? <p className="mt-0.5 text-red-800">{current.reject.doc}</p> : null}
              </div>
            ) : null}
          </section>
        </div>
      ) : (
        <p className="text-[11px] text-muted-foreground">Pick a scenario to step through its given, when and then.</p>
      )}
    </div>
  );
}

export function SliceBoard({
  model,
  selections,
  onSelect,
  focus,
}: {
  model: Model;
  selections: Selection[];
  onSelect: (
    key: string | undefined,
    occId?: string,
    additive?: boolean,
    instanceUid?: string,
    momentKey?: string,
  ) => void;
  focus?: FocusRequest;
}) {
  const issues = useIssues();
  // Persists as ?slice=<namespace>/<slug> so a link opens the same slice.
  const [param, setParam] = useQueryState('slice');
  // Without ?slice=, open the slice of whatever was selected when the view
  // opened. It is read once: a later card click selects an entity on this
  // board and must not move the board under the pointer.
  const [initial] = useState(() => {
    for (const s of selections) {
      if (s.type !== 'entity') continue;
      const hit = sliceForEntity(model, s.key);
      if (hit) return sliceParam(hit);
    }
    return undefined;
  });
  const slice = useMemo(
    () => sliceByParam(model, param) ?? sliceByParam(model, initial) ?? orderedSlices(model)[0],
    [model, param, initial],
  );
  // A stale or mistyped ?slice= must not stay in the URL while another slice shows.
  useEffect(() => {
    if (param && !sliceByParam(model, param)) void setParam(null);
  }, [model, param, setParam]);
  const groups = useMemo(() => sliceGroups(model), [model]);
  const hood = useMemo(() => (slice ? sliceNeighborhood(model, slice) : undefined), [model, slice]);
  const selectedKeys = useMemo(
    () => new Set(selections.filter((s) => s.type === 'entity').map((s) => s.key)),
    [selections],
  );

  const [tab, setTab] = useState<PanelTab>('fields');
  // The traced field survives a slice change, so story playback can follow
  // one field from slice to slice.
  const [traced, setTraced] = useState<string>();
  const [storyboardKey, setStoryboardKey] = useState<string>();
  const [storyPosition, setStoryPosition] = useState(0);
  const [picks, setPicks] = useState(0);
  const [scenario, setScenario] = useState<{ slice: string; index: number; step: number }>();
  const activeScenario = slice && scenario?.slice === slice.entity.key ? scenario : undefined;

  const flow = useMemo(() => (hood && traced ? fieldFlow(model, hood, traced) : undefined), [model, hood, traced]);
  const highlight: Highlight = useMemo(() => {
    if (tab === 'scenarios' && slice && activeScenario) {
      const sc = slice.scenarios[activeScenario.index];
      const step = sc ? scenarioSteps(slice, sc)[activeScenario.step] : undefined;
      if (step) return { kind: 'scenario', keys: scenarioStepKeys(step) };
    }
    return flow ? { kind: 'field', flow } : undefined;
  }, [tab, slice, activeScenario, flow]);

  // A focus request from the drawer or the sidebar shows its entity: on this
  // board when it is here, otherwise on the first slice that names it. The
  // request live at mount is skipped, because the initial slice already
  // follows the selection it came with.
  const handledFocus = useRef(focus?.n);
  const [scrollTo, setScrollTo] = useState<{ key: string; n: number }>();
  useEffect(() => {
    if (!focus || focus.pending || focus.n === handledFocus.current) return;
    handledFocus.current = focus.n;
    if (hood?.cards.some((c) => c.entity?.key === focus.key)) {
      setScrollTo({ key: focus.key, n: focus.n });
      return;
    }
    const target = sliceForEntity(model, focus.key);
    if (!target) return;
    setScrollTo({ key: focus.key, n: focus.n });
    if (target.entity.key !== slice?.entity.key) setParam(sliceParam(target));
  }, [focus, hood, model, slice, setParam]);
  const canvasRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!scrollTo) return;
    const cards = canvasRef.current?.querySelectorAll<HTMLElement>('[data-entity-key]') ?? [];
    const el = [...cards].find((c) => c.dataset.entityKey === scrollTo.key);
    if (!el) {
      // The target slice is on screen and has no card for the key, so no later render adds one.
      if (sliceForEntity(model, scrollTo.key)?.entity.key === slice?.entity.key) setScrollTo(undefined);
      return;
    }
    el.scrollIntoView?.({ block: 'nearest', inline: 'center', behavior: 'smooth' });
    setScrollTo(undefined);
  }, [scrollTo, hood, model, slice]);

  const goToStory = useCallback(
    (index: number) => {
      if (!slice) return;
      const storyboards = storyboardsOf(model, slice);
      const sb = storyboards.find((s) => s.key === storyboardKey) ?? storyboards[0];
      const next = sb ? storySteps(model, sb).slices[index] : undefined;
      if (!next) return;
      // Keep the chosen storyboard when the next slice is listed by several.
      setStoryboardKey(sb.key);
      setStoryPosition(index);
      setParam(sliceParam(next));
    },
    [model, slice, storyboardKey, setParam],
  );
  const goToScenarioStep = useCallback((step: number) => setScenario((cur) => (cur ? { ...cur, step } : cur)), []);

  if (!slice || !hood) {
    return (
      <BoardEmptyState icon={Columns3} title="No slice to show">
        This view shows one slice at a time, and this model has no slices.
      </BoardEmptyState>
    );
  }
  const ownFindings = validationEntities(model, hood).flatMap((e) => issues.for(e));
  const worst = worstSeverity(ownFindings);
  const selectFromPanel = (key: string) => onSelect(key);

  return (
    <div className="flex h-full w-full flex-col pt-12">
      <BoardTopRight>
        <label className="flex items-center gap-1.5 rounded-md border border-border bg-card/95 px-2.5 py-1.5 text-[11px] shadow-sm">
          <span className="text-muted-foreground">Slice</span>
          <select
            aria-label="Slice"
            value={sliceParam(slice)}
            onChange={(e) => {
              setPicks((n) => n + 1);
              setParam(e.target.value);
            }}
            className="max-w-[18rem] cursor-pointer bg-transparent outline-none"
          >
            {groups.map((g) => (
              <optgroup key={g.storyboard?.key ?? 'none'} label={g.storyboard?.title ?? 'In no storyboard'}>
                {g.slices.map((s) => (
                  <option key={s.entity.key} value={sliceParam(s)}>
                    {s.entity.title} · {kindStyle(s.kind).label}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        </label>
      </BoardTopRight>
      <div className="relative min-h-0 flex-1">
        <div
          ref={canvasRef}
          className="absolute inset-0 overflow-auto"
          style={{
            backgroundImage: 'radial-gradient(#d4d4d8 1.2px, transparent 1.2px)',
            backgroundSize: '18px 18px',
          }}
        >
          <SliceBoardCanvas
            model={model}
            hood={hood}
            selectedKeys={selectedKeys}
            highlight={highlight}
            onSelect={(key, additive) => onSelect(key, undefined, additive)}
            onSelectCard={(card, additive) => {
              // The moment is the slice that declares the card, so the
              // Inspector shows the same instance the other boards show.
              if (card.entity)
                onSelect(
                  card.entity.key,
                  undefined,
                  additive,
                  instanceUidOf(card.entity, card.moment),
                  card.moment.key,
                );
            }}
          />
        </div>
        <div className="pointer-events-none absolute bottom-3 right-3 rounded-md border border-border bg-card/95 px-2.5 py-2 text-[10px]">
          {flow && highlight?.kind === 'field' ? (
            <div className="flex items-center gap-3">
              <span className="flex items-center gap-1.5">
                <span className="inline-block h-3 w-3 rounded-sm ring-2 ring-zinc-700 ring-offset-1" />
                holds or passes {flow.name}
              </span>
              <span className="flex items-center gap-1.5">
                <span className="inline-flex h-3 w-3 items-center justify-center rounded-full bg-zinc-700 text-[8px] font-bold text-white">
                  ×
                </span>
                {flow.name} stops here
              </span>
            </div>
          ) : (
            <div className="flex items-center gap-1.5">
              <span className="inline-block h-0.5 w-4 bg-zinc-400" />
              links the slices declare · click a card to inspect it
            </div>
          )}
        </div>
      </div>
      <section
        aria-label="Slice details"
        className="flex h-[34%] min-h-48 shrink-0 flex-col border-t border-border bg-card"
      >
        <Tabs value={tab} onValueChange={(v) => setTab(v as PanelTab)} className="flex min-h-0 flex-1 flex-col">
          <TabsList variant="underline" className="px-3">
            <TabsTab value="fields">Fields</TabsTab>
            <TabsTab value="validation">
              Validation
              {worst ? (
                <span className={cn('rounded-full px-1.5 text-[10px] font-semibold', PILL_STYLE[worst])}>
                  {ownFindings.length}
                </span>
              ) : null}
            </TabsTab>
            <TabsTab value="story">Story</TabsTab>
            <TabsTab value="scenarios">
              Scenarios
              {slice.scenarios.length > 0 ? (
                <span className="rounded-full bg-muted px-1.5 text-[10px] font-semibold">{slice.scenarios.length}</span>
              ) : null}
            </TabsTab>
          </TabsList>
          <TabsPanel value="fields" className="min-h-0 flex-1 overflow-auto p-3">
            <FieldsPanel
              hood={hood}
              model={model}
              traced={traced}
              flow={flow}
              onTrace={setTraced}
              onSelect={selectFromPanel}
            />
          </TabsPanel>
          <TabsPanel value="validation" className="min-h-0 flex-1 overflow-auto p-3">
            <ValidationPanel model={model} hood={hood} onSelect={selectFromPanel} />
          </TabsPanel>
          <TabsPanel value="story" className="min-h-0 flex-1 overflow-auto p-3">
            <StoryPanel
              model={model}
              hood={hood}
              storyboardKey={storyboardKey}
              onStoryboard={setStoryboardKey}
              position={storyPosition}
              onStep={goToStory}
              picks={picks}
            />
          </TabsPanel>
          <TabsPanel value="scenarios" className="min-h-0 flex-1 overflow-auto p-3">
            <ScenariosPanel
              hood={hood}
              scenario={activeScenario?.index}
              step={activeScenario?.step ?? 0}
              onScenario={(index) =>
                setScenario(index === undefined ? undefined : { slice: slice.entity.key, index, step: 0 })
              }
              onStep={goToScenarioStep}
            />
          </TabsPanel>
        </Tabs>
      </section>
    </div>
  );
}

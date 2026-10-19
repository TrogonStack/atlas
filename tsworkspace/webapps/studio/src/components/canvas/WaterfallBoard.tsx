// The waterfall view (see lib/waterfall.ts): rows of steps against the
// slice timeline. It is a scrolling list, not a canvas. Rows read like a
// table, and every row has a fixed height, so the cable gutter places each
// cable from row positions without measuring the DOM.

import { ChevronRight, Layers } from 'lucide-react';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { Selection } from '@/components/canvas/Board';
import { kindStyle } from '@/components/canvas/StickyNode';
import { IssueBadge } from '@/components/IssueBadge';
import { useIssues } from '@/components/IssuesProvider';
import { Button } from '@/components/ui/button';
import { BoardEmptyState } from '@/components/ui/empty-state';
import type { Issue, IssueIndex, Severity } from '@/lib/issues';
import type { Entity, EntityKind, Model } from '@/lib/model';
import { cn } from '@/lib/utils';
import {
  buildWaterfall,
  type CodeBundle,
  type ColumnSummary,
  type RowTarget,
  type Waterfall,
  type WaterfallCable,
  type WaterfallGroup,
  type WaterfallRow,
} from '@/lib/waterfall';
import type { FocusRequest } from './useFocusNode';

type Mode = 'storyboard' | 'type';

const HEAD_H = 44;
const ROW_H = 30;
const GUTTER_W = 112;
const COL_MIN_W = 16;
const MAX_CHIPS = 2;

const RANK: Record<Severity, number> = { error: 0, warning: 1, info: 2 };

// IssueBadge takes its severity from the first finding, so a roll-up over
// several entities has to put the worst one first again.
function worstFirst(issues: Issue[]): Issue[] {
  return [...issues].sort((a, b) => RANK[a.severity] - RANK[b.severity]);
}

function groupIssues(issues: IssueIndex, group: WaterfallGroup): Issue[] {
  const entities = group.entity ? [group.entity, ...group.members] : group.members;
  return worstFirst(entities.flatMap((e) => issues.for(e)));
}

function KindSwatch({ kind, dashed }: { kind?: EntityKind; dashed?: boolean }) {
  const style = kind ? kindStyle(kind) : undefined;
  const Icon = style?.icon ?? Layers;
  return (
    <span
      title={style?.label}
      className={cn(
        'grid h-4 w-4 shrink-0 place-items-center rounded-sm border',
        style ? [style.bg, style.border] : 'border-zinc-300 bg-card',
        dashed && 'border-dashed bg-card',
      )}
    >
      <Icon aria-hidden className="h-2.5 w-2.5 text-zinc-700" />
      {style ? <span className="sr-only">{style.label}</span> : null}
    </span>
  );
}

const at = (col: number, n: number) => ({
  left: `${(col / n) * 100}%`,
  width: `${100 / n}%`,
});

// The track cell: one column per slice, with the selection's columns lit.
function Track({ n, lit, children }: { n: number; lit: Set<number>; children: React.ReactNode }) {
  return (
    <div
      className="relative h-[22px] border-r border-border"
      style={{
        backgroundImage: 'linear-gradient(to right, var(--border) 1px, transparent 1px)',
        backgroundSize: `${100 / n}% 100%`,
      }}
    >
      {[...lit].map((col) => (
        <span key={`lit-${col}`} className="absolute inset-y-0 bg-primary/10" style={at(col, n)} />
      ))}
      {children}
    </div>
  );
}

function SpanBar({ n, span, dashed }: { n: number; span?: { start: number; end: number }; dashed?: boolean }) {
  if (!span || span.end <= span.start) return null;
  return (
    <span
      className={cn(
        'absolute top-1/2 -translate-y-1/2',
        dashed ? 'h-0 border-t border-dashed border-zinc-400' : 'h-[2px] rounded bg-zinc-300',
      )}
      style={{
        left: `${((span.start + 0.5) / n) * 100}%`,
        width: `${((span.end - span.start) / n) * 100}%`,
      }}
    />
  );
}

function RowTrack({ n, row, lit }: { n: number; row: WaterfallRow; lit: Set<number> }) {
  const style = kindStyle(row.entity.kind);
  const ghost = row.type !== 'step';
  return (
    <Track n={n} lit={lit}>
      <SpanBar n={n} span={row.span} dashed={ghost} />
      {row.cols.map((col) => (
        <span key={col} className="absolute inset-y-[4px] px-[2px]" style={at(col, n)}>
          <span
            className={cn('block h-full rounded-sm border', style.border, ghost ? 'border-dashed bg-card' : style.bg)}
          />
        </span>
      ))}
    </Track>
  );
}

function GroupTrack({ n, group, lit }: { n: number; group: WaterfallGroup; lit: Set<number> }) {
  return (
    <Track n={n} lit={lit}>
      <SpanBar n={n} span={group.span} />
      {group.columns.map((c: ColumnSummary) => (
        <span
          key={c.col}
          title={`Slice ${c.col + 1}: ${c.total} ${c.total === 1 ? 'step' : 'steps'}`}
          className="absolute inset-y-[3px] px-[2px]"
          style={at(c.col, n)}
        >
          <span className="relative flex h-full flex-col overflow-hidden rounded-sm border border-zinc-400 bg-card">
            {c.kinds.map((k) => (
              <span key={k.kind} className={cn('block', kindStyle(k.kind).bg)} style={{ flexGrow: k.count }} />
            ))}
            {c.total > 1 ? (
              <span className="absolute inset-0 grid place-items-center font-mono text-[9px] font-semibold tabular-nums text-zinc-800">
                {c.total}
              </span>
            ) : null}
          </span>
        </span>
      ))}
    </Track>
  );
}

function CodeCell({ bundles }: { bundles: CodeBundle[] }) {
  const shown = bundles.slice(0, MAX_CHIPS);
  const more = bundles.slice(MAX_CHIPS);
  const tip = (b: CodeBundle) =>
    `${b.refs} ${b.refs === 1 ? 'reference' : 'references'} to ${b.files.length} ${b.files.length === 1 ? 'file' : 'files'} in ${b.label}:\n${b.files.join('\n')}`;
  return (
    <div className="flex min-w-0 items-center gap-1">
      {shown.map((b) => (
        <span
          key={b.key}
          title={tip(b)}
          className="inline-flex min-w-0 items-center gap-1 rounded border border-border bg-card px-1.5 font-mono text-[10px] leading-4"
        >
          {/* The folder is what tells chips apart. The tooltip names the repo. */}
          <span className="truncate">{b.folder || b.repo}</span>
          <span className="font-sans font-semibold text-muted-foreground">{b.refs}</span>
        </span>
      ))}
      {more.length > 0 ? (
        <span
          title={more.map((b) => `${b.label} (${b.refs})`).join('\n')}
          className="shrink-0 rounded border border-border bg-card px-1.5 font-mono text-[10px] leading-4 text-muted-foreground"
        >
          +{more.length}
        </span>
      ) : null}
    </div>
  );
}

function Tag({ children, title }: { children: React.ReactNode; title?: string }) {
  return (
    <span
      title={title}
      className="shrink-0 whitespace-nowrap rounded-full border border-border px-1.5 text-[10px] leading-4 text-muted-foreground"
    >
      {children}
    </span>
  );
}

function rowTag(row: WaterfallRow, mode: Mode): React.ReactNode {
  if (row.type === 'join') {
    return (
      <Tag title={`${row.targetTitle ?? 'Another group'} lists this step. Click to go there.`}>
        joins {row.targetTitle}
      </Tag>
    );
  }
  if (row.type === 'above')
    return <Tag title="This group already lists this step. Click to go there.">shown above</Tag>;
  if (mode === 'type')
    return <Tag title="Happens-next links out and in">{`${row.linksOut} out · ${row.linksIn} in`}</Tag>;
  if (row.joinedBy.length === 1)
    return <Tag title="Another group reaches this step too">also in {row.joinedBy[0]}</Tag>;
  if (row.joinedBy.length > 1) {
    return <Tag title={`Also reached from: ${row.joinedBy.join(', ')}`}>also in {row.joinedBy.length} groups</Tag>;
  }
  return null;
}

interface Placed {
  key: string;
  top: number;
  group: WaterfallGroup;
  row?: WaterfallRow;
}

function CableGutter({
  cables,
  groups,
  placed,
  open,
  selected,
  height,
  onOpenBoth,
}: {
  cables: WaterfallCable[];
  groups: WaterfallGroup[];
  placed: Placed[];
  open: ReadonlySet<string>;
  selected: Set<string>;
  height: number;
  onOpenBoth: (cable: WaterfallCable) => void;
}) {
  const markerBase = useId();
  const headerY = new Map<string, number>();
  const memberY = new Map<string, number>();
  for (const p of placed) {
    if (p.row) memberY.set(p.row.entity.key, p.top + ROW_H / 2);
    else headerY.set(p.group.key, p.top + HEAD_H / 2);
  }
  const titleOf = new Map(groups.map((g) => [g.key, g.lane ? `${g.title} (${g.lane})` : g.title]));
  const colors = [...new Set(cables.map((c) => c.color))];
  const markerId = (color: string) => `${markerBase}-arrow-${colors.indexOf(color)}`;
  const maxN = Math.max(1, ...cables.map((c) => c.links.length));
  const trunkX = GUTTER_W - 16;
  const endX = GUTTER_W - 2;
  const avg = (ys: number[]) => ys.reduce((s, y) => s + y, 0) / ys.length;

  const drawn = cables.flatMap((c) => {
    // An open group splits its end of the cable into one fiber per member
    // that carries a link. A closed group takes the whole cable at its header.
    const ends = (group: string, side: 'from' | 'to') =>
      (open.has(group)
        ? [...new Set(c.links.map((l) => l[side].key))].map((k) => memberY.get(k))
        : [headerY.get(group)]
      ).filter((y): y is number => y !== undefined);
    const ya = ends(c.from, 'from').map((y) => y - 3);
    const yb = ends(c.to, 'to').map((y) => y + 3);
    if (ya.length === 0 || yb.length === 0) return [];
    const ma = avg(ya);
    const mb = avg(yb);
    const bendX = Math.max(8, trunkX - 10 - Math.min(trunkX - 18, 10 + Math.abs(mb - ma) * 0.3));
    const n = c.links.length;
    const w = 1.5 + (5.5 * n) / maxN;
    const hot = c.links.some((l) => selected.has(l.from.key) || selected.has(l.to.key));
    return [{ c, ya, yb, ma, mb, bendX, n, w, hot }];
  });

  return (
    <>
      <svg
        aria-hidden
        data-testid="waterfall-cables"
        className="pointer-events-none absolute left-0 top-0 z-[1] overflow-visible"
        width={GUTTER_W}
        height={height}
      >
        <defs>
          {colors.map((color) => (
            <marker
              key={color}
              id={markerId(color)}
              viewBox="0 0 8 8"
              refX="7"
              refY="4"
              markerWidth="6"
              markerHeight="6"
              markerUnits="userSpaceOnUse"
              orient="auto"
            >
              <path d="M0,0 L8,4 L0,8 z" fill={color} />
            </marker>
          ))}
        </defs>
        {drawn.map(({ c, ya, yb, ma, mb, bendX, w, hot }) => (
          // Selection raises a cable's opacity and keeps its color, so the
          // flow it carries still reads while it stands out.
          <g key={c.key} data-cable={c.key} opacity={hot ? 1 : 0.5} stroke={c.color} fill="none" strokeLinecap="round">
            <path strokeWidth={w} d={`M${trunkX},${ma} C${bendX},${ma} ${bendX},${mb} ${trunkX},${mb}`} />
            {ya.map((y) => (
              <path
                key={`a${y}`}
                strokeWidth={ya.length > 1 ? Math.max(1, w / ya.length) : w}
                d={`M${trunkX},${ma} C${trunkX + 7},${ma} ${endX - 8},${y} ${endX - 2},${y}`}
              />
            ))}
            {yb.map((y) => (
              <path
                key={`b${y}`}
                strokeWidth={yb.length > 1 ? Math.max(1, w / yb.length) : w}
                markerEnd={`url(#${markerId(c.color)})`}
                d={`M${trunkX},${mb} C${trunkX + 7},${mb} ${endX - 10},${y} ${endX - 6},${y}`}
              />
            ))}
          </g>
        ))}
      </svg>
      {drawn.map(({ c, ma, mb, bendX, n, hot }) => {
        const label = `${n} ${n === 1 ? 'link' : 'links'} from ${titleOf.get(c.from)} to ${titleOf.get(c.to)}`;
        return (
          <button
            key={c.key}
            type="button"
            title={`${label}. Click to open or close both ends.`}
            aria-label={`${label}. Open or close both ends.`}
            onClick={() => onOpenBoth(c)}
            className={cn(
              'absolute z-[2] -translate-x-full -translate-y-1/2 rounded bg-card/90 px-0.5 font-mono text-[10px] leading-3 font-semibold tabular-nums hover:text-foreground',
              hot ? 'text-foreground' : 'text-muted-foreground',
            )}
            style={{ left: trunkX + 0.75 * (bendX - trunkX) - 3, top: (ma + mb) / 2 }}
          >
            {n}
          </button>
        );
      })}
    </>
  );
}

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

function chainMeta(g: WaterfallGroup): string {
  if (g.rows.length === 0) return 'No steps yet';
  const parts = [count(g.members.length, 'step', 'steps')];
  const joins = g.rows.filter((r) => r.type === 'join').length;
  if (joins > 0) parts.push(`${joins} shared with other groups`);
  if (g.span) parts.push(`slices ${g.span.start + 1} to ${g.span.end + 1}`);
  return parts.join(' · ');
}

function typeMeta(g: WaterfallGroup): string {
  return `${count(g.members.length, 'entity', 'entities')} · ${count(g.linksOut, 'link', 'links')} out · ${g.linksIn} in`;
}

function homeOf(wf: Waterfall, mode: Mode, key: string): RowTarget | undefined {
  if (mode === 'type') return wf.typeHome.get(key);
  const home = wf.storyboardHome.get(key);
  if (home) return home;
  // A selected storyboard lands on its own chain header.
  const chain = wf.storyboards.find((g) => g.entity?.key === key);
  return chain ? { group: chain.key, row: chain.key } : undefined;
}

export function WaterfallBoard({
  model,
  selections,
  onSelect,
  focus,
}: {
  model: Model;
  selections: Selection[];
  onSelect: (key: string | undefined, occId?: string, additive?: boolean) => void;
  focus?: FocusRequest;
}) {
  const wf = useMemo(() => buildWaterfall(model), [model]);
  const issues = useIssues();
  const [mode, setMode] = useState<Mode>('storyboard');
  const [openByMode, setOpenByMode] = useState<Record<Mode, ReadonlySet<string>>>({
    storyboard: new Set(),
    type: new Set(),
  });
  const open = openByMode[mode];
  const groups = mode === 'storyboard' ? wf.storyboards : wf.types;
  const n = Math.max(1, wf.slices.length);

  const selected = useMemo(
    () => new Set(selections.filter((s) => s.type === 'entity').map((s) => s.key)),
    [selections],
  );
  const lit = useMemo(() => {
    const cols = new Set<number>();
    for (const key of selected) for (const col of wf.occurrences.get(key) ?? []) cols.add(col);
    return cols;
  }, [selected, wf]);

  const setOpen = useCallback((m: Mode, update: (prev: ReadonlySet<string>) => ReadonlySet<string>) => {
    setOpenByMode((prev) => ({ ...prev, [m]: update(prev[m]) }));
  }, []);
  const toggle = (key: string) =>
    setOpen(mode, (prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  // Row elements by row key (group headers by group key), so a jump can
  // scroll to a row once its group is open.
  const rowEls = useRef(new Map<string, HTMLElement>());
  const register = (key: string) => (el: HTMLElement | null) => {
    if (el) rowEls.current.set(key, el);
    else rowEls.current.delete(key);
  };
  const [scrollTarget, setScrollTarget] = useState<{ row: string; n: number }>();
  useEffect(() => {
    if (!scrollTarget) return;
    rowEls.current.get(scrollTarget.row)?.scrollIntoView?.({ block: 'center' });
  }, [scrollTarget]);
  const goTo = useCallback(
    (m: Mode, target: RowTarget) => {
      setOpen(m, (prev) => (prev.has(target.group) ? prev : new Set([...prev, target.group])));
      setScrollTarget((prev) => ({ row: target.row, n: (prev?.n ?? 0) + 1 }));
    },
    [setOpen],
  );

  // focus.n and focus.key are the intentional re-triggers, as in
  // useFocusNode: a rebuilt model must not move the list on its own.
  const focusKey = focus && !focus.pending ? focus.key : undefined;
  const focusN = focus?.n;
  const latest = useRef({ wf, mode });
  latest.current = { wf, mode };
  useEffect(() => {
    if (focusKey === undefined || focusN === undefined) return;
    const { wf: current, mode: m } = latest.current;
    const target = homeOf(current, m, focusKey);
    if (target) goTo(m, target);
  }, [focusKey, focusN, goTo]);

  // The selection stays in view across groupings: its home row in the new
  // grouping opens and scrolls into view.
  const switchMode = (m: Mode) => {
    if (m === mode) return;
    setMode(m);
    const [key] = selected;
    const target = key ? homeOf(wf, m, key) : undefined;
    if (target) goTo(m, target);
  };

  const select = (e: React.MouseEvent, entity: Entity) =>
    onSelect(entity.key, undefined, e.metaKey || e.ctrlKey || e.shiftKey);

  const { placed, height } = useMemo(() => {
    const out: Placed[] = [];
    let y = 0;
    for (const g of groups) {
      out.push({ key: g.key, top: y, group: g });
      y += HEAD_H;
      if (!open.has(g.key)) continue;
      for (const r of g.rows) {
        out.push({ key: r.key, top: y, group: g, row: r });
        y += ROW_H;
      }
    }
    return { placed: out, height: y };
  }, [groups, open]);

  if (wf.storyboards.length === 0 && wf.types.length === 0) {
    return (
      <BoardEmptyState icon={Layers} title="No steps to list">
        No slice in this model names a UI, command, event, read model or automation yet.
      </BoardEmptyState>
    );
  }

  const gutter = mode === 'type';
  const columns = [
    gutter ? `${GUTTER_W}px` : null,
    'minmax(200px, 300px)',
    'minmax(80px, 120px)',
    `minmax(${n * COL_MIN_W}px, 1fr)`,
    wf.hasCode ? 'minmax(160px, 280px)' : null,
  ]
    .filter(Boolean)
    .join(' ');
  const minWidth = (gutter ? GUTTER_W : 12) + 200 + 80 + n * COL_MIN_W + (wf.hasCode ? 160 : 0) + 4 * 10 + 12;
  const gridClass = cn('grid items-center gap-x-2.5 pr-3', !gutter && 'pl-3');
  const gridStyle = { gridTemplateColumns: columns };

  const steps = wf.storyboards.reduce((sum, g) => sum + g.members.length, 0);
  const summary =
    mode === 'storyboard'
      ? `${wf.storyboards.filter((g) => g.entity).length} storyboards · ${steps} steps · ${wf.shared} shared`
      : `${wf.types.length} object types · ${wf.cables.length} cables carry ${wf.links.length} links`;
  const hint =
    mode === 'storyboard'
      ? 'A bar runs from a step to the last slice that reads it. A dashed row points to where the step is listed.'
      : 'A cable carries every link between two object types. Open a type to split its end into one fiber per entity.';

  return (
    // The controls sit in the list's own bar, not in the shared top-right
    // row: a list has no canvas to float them over, and at narrow widths
    // the shared row has no room left beside the view switcher.
    <div className="absolute inset-x-0 bottom-0 top-12 flex flex-col bg-background" data-testid="waterfall">
      <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1.5 px-3 pb-2 text-[11px]">
        <fieldset aria-label="Group rows" className="inline-flex h-7 items-center rounded-md bg-muted p-[3px]">
          {(
            [
              ['storyboard', 'By storyboard'],
              ['type', 'By object type'],
            ] as const
          ).map(([m, label]) => (
            <button
              key={m}
              type="button"
              aria-pressed={mode === m}
              onClick={() => switchMode(m)}
              className={cn(
                'h-full whitespace-nowrap rounded px-2.5 text-xs font-medium text-muted-foreground hover:text-foreground',
                mode === m && 'bg-background text-foreground shadow-sm',
              )}
            >
              {label}
            </button>
          ))}
        </fieldset>
        <div className="flex items-center gap-1">
          <Button
            variant="outline"
            size="sm"
            className="h-7 px-2"
            onClick={() => setOpen(mode, () => new Set(groups.map((g) => g.key)))}
          >
            Open all
          </Button>
          <Button variant="outline" size="sm" className="h-7 px-2" onClick={() => setOpen(mode, () => new Set())}>
            Close all
          </Button>
        </div>
        <span className="font-semibold text-foreground">{summary}</span>
        <span className="text-muted-foreground">{hint}</span>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <div style={{ minWidth }}>
          <div
            className={cn(
              gridClass,
              'sticky top-0 z-[2] h-7 border-y border-border bg-card text-[10px] font-semibold uppercase tracking-wide text-muted-foreground',
            )}
            style={gridStyle}
          >
            {gutter ? <span /> : null}
            <span className="truncate">{mode === 'storyboard' ? 'Storyboard, in slice order' : 'Object type'}</span>
            <span>Lane</span>
            <div
              className="grid text-center font-mono text-[9px] font-normal normal-case tracking-normal tabular-nums"
              style={{ gridTemplateColumns: `repeat(${n}, minmax(0, 1fr))` }}
            >
              {wf.slices.map((s, i) => (
                <span
                  key={s.entity.key}
                  title={`Slice ${i + 1}: ${s.entity.title}`}
                  className={cn('mx-px rounded-sm', lit.has(i) && 'bg-foreground text-background')}
                >
                  {i + 1}
                </span>
              ))}
            </div>
            {wf.hasCode ? <span className="truncate">Code, by folder</span> : null}
          </div>
          <div className="relative" style={{ height }}>
            {gutter ? (
              <CableGutter
                cables={wf.cables}
                groups={wf.types}
                placed={placed}
                open={open}
                selected={selected}
                height={height}
                onOpenBoth={(c) =>
                  setOpen('type', (prev) => {
                    const both = prev.has(c.from) && prev.has(c.to);
                    const next = new Set(prev);
                    for (const k of [c.from, c.to]) {
                      if (both) next.delete(k);
                      else next.add(k);
                    }
                    return next;
                  })
                }
              />
            ) : null}
            {placed.map((p) => {
              const g = p.group;
              if (!p.row) {
                const isOpen = open.has(g.key);
                const isChain = mode === 'storyboard';
                const isSelected = Boolean(g.entity && selected.has(g.entity.key));
                const meta = isChain ? chainMeta(g) : typeMeta(g);
                const headIssues = groupIssues(issues, g);
                return (
                  <div
                    key={p.key}
                    ref={register(g.key)}
                    data-row-type="group"
                    data-group={g.key}
                    className={cn(
                      gridClass,
                      'absolute inset-x-0 border-b border-border bg-muted/60',
                      isSelected && 'bg-accent shadow-[inset_3px_0_0_var(--ring)]',
                    )}
                    style={{ ...gridStyle, top: p.top, height: HEAD_H }}
                  >
                    {gutter ? <span /> : null}
                    <div className="flex min-w-0 flex-col justify-center">
                      <div className="flex min-w-0 items-center gap-1.5">
                        <button
                          type="button"
                          aria-expanded={isOpen}
                          aria-label={`${isOpen ? 'Close' : 'Open'} ${g.title}`}
                          onClick={() => toggle(g.key)}
                          className="grid h-5 w-5 shrink-0 place-items-center rounded border border-border bg-card text-foreground hover:bg-accent"
                        >
                          <ChevronRight aria-hidden className={cn('h-3 w-3', isOpen && 'rotate-90')} />
                        </button>
                        <KindSwatch kind={isChain ? g.entity?.kind : g.kind} dashed={isChain && !g.entity} />
                        <button
                          type="button"
                          onClick={(e) => (g.entity ? select(e, g.entity) : toggle(g.key))}
                          className="min-w-0 truncate text-left text-[12px] font-semibold text-foreground hover:underline"
                        >
                          {g.title}
                        </button>
                        <IssueBadge issues={headIssues} />
                      </div>
                      <div className="truncate pl-[44px] text-[10px] text-muted-foreground">{meta}</div>
                    </div>
                    <div className="truncate text-[10px] text-muted-foreground" title={g.lane}>
                      {g.lane ?? ''}
                    </div>
                    <GroupTrack n={n} group={g} lit={lit} />
                    {wf.hasCode ? <CodeCell bundles={g.bundles} /> : null}
                  </div>
                );
              }
              const r = p.row;
              const isSelected = selected.has(r.entity.key);
              const target = r.target;
              return (
                <div
                  key={p.key}
                  ref={register(r.key)}
                  data-row-type={r.type}
                  data-entity-key={r.entity.key}
                  data-selected={isSelected || undefined}
                  className={cn(
                    gridClass,
                    'absolute inset-x-0 border-b border-border bg-card',
                    isSelected && 'bg-accent shadow-[inset_3px_0_0_var(--ring)]',
                  )}
                  style={{ ...gridStyle, top: p.top, height: ROW_H }}
                >
                  {gutter ? <span /> : null}
                  <div className="flex min-w-0 items-center gap-1.5" style={{ paddingLeft: 26 + r.depth * 12 }}>
                    <KindSwatch kind={r.entity.kind} dashed={r.type !== 'step'} />
                    <button
                      type="button"
                      onClick={(e) => {
                        select(e, r.entity);
                        if (target) goTo(mode, target);
                      }}
                      className={cn(
                        'min-w-0 truncate text-left text-[12px] hover:underline',
                        r.type === 'step' ? 'font-medium text-foreground' : 'text-muted-foreground',
                      )}
                    >
                      {r.entity.title}
                    </button>
                    {rowTag(r, mode)}
                    {r.type === 'step' ? <IssueBadge issues={issues.for(r.entity)} /> : null}
                  </div>
                  <div className="truncate text-[10px] text-muted-foreground" title={r.lane}>
                    {r.lane}
                  </div>
                  <RowTrack n={n} row={r} lit={lit} />
                  {wf.hasCode ? <CodeCell bundles={r.bundles} /> : null}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}

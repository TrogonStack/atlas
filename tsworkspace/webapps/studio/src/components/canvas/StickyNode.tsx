import { Handle, type NodeProps, Position } from '@xyflow/react';
import {
  Activity,
  Bell,
  Boxes,
  Clapperboard,
  Cog,
  Columns3,
  Database,
  Globe,
  Info,
  Layers,
  ListChecks,
  Megaphone,
  MonitorSmartphone,
  OctagonAlert,
  Split,
  Target,
  Terminal,
  TriangleAlert,
  User,
  Zap,
} from 'lucide-react';
import { GlyphIcon } from '@/components/canvas/GlyphIcon';
import { IssueBadge } from '@/components/IssueBadge';
import { useIssues } from '@/components/IssuesProvider';
import { type GapHeaderData, type LaneData, NODE_H, NODE_W, type SliceHeaderData, type StickyData } from '@/lib/layout';
import { type EntityKind, orphanOf, upstreamContextsOf } from '@/lib/model';
import { cn } from '@/lib/utils';

// Branch review badges (Phase 3: Studio), see `StickyData.branchStatus` in
// layout.ts for how this gets stamped onto a rendered node.
const BRANCH_STATUS_STYLE: Record<string, { label: string; className: string }> = {
  added: { label: 'added', className: 'border-emerald-400 bg-emerald-100 text-emerald-800' },
  changed: { label: 'changed', className: 'border-sky-400 bg-sky-100 text-sky-800' },
  deleted: { label: 'deleted', className: 'border-zinc-400 bg-zinc-200 text-zinc-700' },
  conflict: { label: 'conflict', className: 'border-red-400 bg-red-100 text-red-800' },
};

const KIND_STYLE: Record<
  string,
  { bg: string; border: string; label: string; icon: React.ComponentType<{ className?: string }> }
> = {
  event: { bg: 'bg-orange-200', border: 'border-orange-400', label: 'Event', icon: Zap },
  command: { bg: 'bg-sky-200', border: 'border-sky-400', label: 'Command', icon: Terminal },
  readModel: { bg: 'bg-emerald-200', border: 'border-emerald-400', label: 'Read Model', icon: Database },
  processor: { bg: 'bg-violet-200', border: 'border-violet-400', label: 'Automation', icon: Cog },
  ui: { bg: 'bg-white', border: 'border-zinc-300', label: 'UI', icon: MonitorSmartphone },
  persona: { bg: 'bg-yellow-100', border: 'border-yellow-300', label: 'Persona', icon: User },
  swimlane: { bg: 'bg-yellow-50', border: 'border-yellow-300', label: 'Swimlane', icon: User },
  commandSlice: { bg: 'bg-sky-50', border: 'border-sky-300', label: 'Command Slice', icon: Columns3 },
  readModelSlice: { bg: 'bg-emerald-50', border: 'border-emerald-300', label: 'Read Model Slice', icon: Columns3 },
  automationSlice: { bg: 'bg-violet-50', border: 'border-violet-300', label: 'Automation Slice', icon: Columns3 },
  uiSlice: { bg: 'bg-zinc-50', border: 'border-zinc-300', label: 'UI Slice', icon: Columns3 },
  storyboard: { bg: 'bg-zinc-100', border: 'border-zinc-300', label: 'Storyboard', icon: Clapperboard },
  eventModel: { bg: 'bg-zinc-200', border: 'border-zinc-400', label: 'Event Model', icon: Layers },
  component: { bg: 'bg-zinc-100', border: 'border-zinc-400', label: 'Component', icon: Boxes },
  externalSystem: { bg: 'bg-fuchsia-100', border: 'border-fuchsia-300', label: 'External System', icon: Globe },
  tracker: { bg: 'bg-stone-100', border: 'border-stone-400', label: 'Tracker', icon: ListChecks },
  boundedContext: { bg: 'bg-fuchsia-50', border: 'border-fuchsia-300', label: 'Bounded Context', icon: Globe },
  domain: { bg: 'bg-indigo-50', border: 'border-indigo-300', label: 'Domain', icon: Globe },
  subdomain: { bg: 'bg-indigo-50', border: 'border-indigo-200', label: 'Subdomain', icon: Boxes },
  schema: { bg: 'bg-slate-50', border: 'border-slate-300', label: 'Schema', icon: ListChecks },
  project: { bg: 'bg-cyan-50', border: 'border-cyan-300', label: 'Project', icon: Boxes },
  screen: { bg: 'bg-teal-50', border: 'border-teal-300', label: 'Screen', icon: MonitorSmartphone },
  term: { bg: 'bg-lime-50', border: 'border-lime-300', label: 'Term', icon: Info },
  ambiguity: { bg: 'bg-rose-50', border: 'border-rose-300', label: 'Ambiguity', icon: Layers },
  serviceLevelIndicator: {
    bg: 'bg-amber-50',
    border: 'border-amber-300',
    label: 'Service Level Indicator',
    icon: Activity,
  },
  serviceLevelObjective: {
    bg: 'bg-amber-100',
    border: 'border-amber-400',
    label: 'Service Level Objective',
    icon: Target,
  },
  alertPolicy: { bg: 'bg-red-50', border: 'border-red-300', label: 'Alert Policy', icon: Bell },
  alertNotificationTarget: {
    bg: 'bg-red-100',
    border: 'border-red-400',
    label: 'Alert Notification Target',
    icon: Megaphone,
  },
  typeLibrary: { bg: 'bg-slate-100', border: 'border-slate-400', label: 'Type Library', icon: ListChecks },
};

export function kindStyle(kind: EntityKind) {
  return (
    KIND_STYLE[kind] ?? {
      bg: 'bg-zinc-100',
      border: 'border-zinc-300',
      label: kind,
      icon: Layers,
    }
  );
}

const handleClass = '!h-1.5 !w-1.5 !border-0 !bg-zinc-400/70';

interface PortSpec {
  id: string;
  type: 'source' | 'target';
  position: Position;
}

// Event Modeling fixes each kind's ports: UIs connect only through the
// bottom, events only through the top, commands receive on top (UI,
// processor) and emit events out the bottom, read models receive events on
// the bottom and feed UIs/processors out the top, processors connect only
// through the bottom.
const KIND_PORTS: Record<string, PortSpec[]> = {
  ui: [
    { id: 'bt', type: 'target', position: Position.Bottom },
    { id: 'bs', type: 'source', position: Position.Bottom },
  ],
  event: [
    { id: 'tt', type: 'target', position: Position.Top },
    { id: 'ts', type: 'source', position: Position.Top },
  ],
  command: [
    { id: 'tt', type: 'target', position: Position.Top },
    { id: 'bs', type: 'source', position: Position.Bottom },
  ],
  readModel: [
    { id: 'bt', type: 'target', position: Position.Bottom },
    { id: 'ts', type: 'source', position: Position.Top },
  ],
  processor: [
    { id: 'bt', type: 'target', position: Position.Bottom },
    { id: 'bs', type: 'source', position: Position.Bottom },
  ],
};

export function StickyCardBody({
  entity,
  synthetic,
  roads,
  instance,
  moment,
  noEmittedEvents,
  multipleIssuers,
  branchStatus,
}: {
  entity: StickyData['entity'];
  synthetic?: boolean;
  roads?: { title: string; doc: string }[];
  instance?: string;
  moment?: StickyData['entity'];
  noEmittedEvents?: boolean;
  multipleIssuers?: boolean;
  branchStatus?: string;
}) {
  const style = kindStyle(entity.kind);
  const Icon = style.icon;
  const orphan = orphanOf(entity);
  const issues = useIssues().for(entity);
  // Boundary views take the boundary color: fed by a third party
  // (externalSource) or translated from another bounded context (cross-
  // namespace source_events); same shape, different trust.
  const upstream = upstreamContextsOf(entity);
  const boundary = entity.externalSource
    ? `External: fed by ${entity.externalSource.system}`
    : upstream.length > 0
      ? `Translated from context: ${upstream.join(', ')}`
      : entity.calls && entity.calls.length > 0
        ? `Calls external: ${entity.calls.map((c) => c.slug).join(', ')}`
        : undefined;
  return (
    <>
      <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-zinc-600">
        <span title={boundary} className="flex shrink-0">
          <Icon className={cn('h-3 w-3', boundary && 'text-fuchsia-600')} />
        </span>
        {style.label}
        {synthetic ? <span className="rounded bg-zinc-200 px-1 text-[8px] text-zinc-700">implicit</span> : null}
        {orphan !== undefined ? (
          <span
            title={orphan || 'OrphanAnnotation present but doc is empty: ORPHAN_DOC_EMPTY'}
            className="inline-flex items-center gap-0.5 rounded border border-amber-400 bg-amber-100 px-1 text-[8px] font-bold uppercase text-amber-800"
          >
            <TriangleAlert aria-hidden className="h-2.5 w-2.5" />
            orphan
          </span>
        ) : null}
        {noEmittedEvents ? (
          <span
            title="COMMAND_NO_EMITTED_EVENTS: no CommandSlice emits events for this command; pending work, add a slice or remove the command"
            className="inline-flex items-center gap-0.5 rounded border border-amber-400 bg-amber-100 px-1 text-[8px] font-bold uppercase text-amber-800"
          >
            <TriangleAlert aria-hidden className="h-2.5 w-2.5" />
            pending
          </span>
        ) : null}
        {multipleIssuers ? (
          <span
            title="COMMAND_MULTIPLE_ISSUERS: this command is issued by more than one UI or processor; the trigger is ambiguous. Split the command, or merge the issuers."
            className="inline-flex items-center gap-0.5 rounded border border-red-400 bg-red-100 px-1 text-[8px] font-bold uppercase text-red-800"
          >
            <OctagonAlert aria-hidden className="h-2.5 w-2.5" />
            multi-issuer
          </span>
        ) : null}
        {branchStatus && BRANCH_STATUS_STYLE[branchStatus] ? (
          <span
            title={`This entity is "${BRANCH_STATUS_STYLE[branchStatus].label}" on the active branch: see the branch review drawer for details`}
            className={cn(
              'rounded border px-1 text-[8px] font-bold uppercase',
              BRANCH_STATUS_STYLE[branchStatus].className,
            )}
          >
            {BRANCH_STATUS_STYLE[branchStatus].label}
          </span>
        ) : null}
        <IssueBadge issues={issues} />
        {roads && roads.length > 0 ? (
          <span
            title={roads.map((r) => `opens: ${r.title}${r.doc ? ` (when ${r.doc})` : ''}`).join('\n')}
            className="inline-flex shrink-0 items-center gap-0.5 rounded border border-amber-300 bg-amber-100 px-1 text-[8px] font-bold normal-case text-amber-800"
          >
            <Split aria-hidden className="h-2.5 w-2.5" />
            {roads.length}
          </span>
        ) : null}
        <span className="ml-auto flex shrink-0 items-center gap-1 font-mono text-[9px] text-zinc-500">
          v{entity.id.version}
          {instance ? (
            <span
              title={
                moment
                  ? `identity: ${entity.id.namespace}/${entity.id.slug} @ ${moment.id.slug}; ${instance} is only its position in this view`
                  : `moment ${instance} of this type: same type, different instance`
              }
              className="text-zinc-400"
            >
              {instance}
            </span>
          ) : null}
        </span>
      </div>
      <div className="mt-1 text-[13px] font-semibold leading-snug text-zinc-900">{entity.title}</div>
      {/* At-a-glance only: the slug. Full identity (ns + defining slice) and
          the doc live in the tooltip and the drawer. */}
      <div
        title={`${entity.id.namespace}/${entity.id.slug}${moment ? ` @ ${moment.id.slug}` : ''}`}
        className="truncate font-mono text-[10px] text-zinc-500"
      >
        {entity.id.slug}
      </div>
    </>
  );
}

export function StickyNode({ data, selected }: NodeProps & { data: StickyData }) {
  const { entity, synthetic, noEmittedEvents, multipleIssuers, branchStatus } = data;
  const style = kindStyle(entity.kind);
  const ports = KIND_PORTS[entity.kind] ?? [];
  const orphan = orphanOf(entity);
  return (
    <div
      style={{ width: NODE_W, height: NODE_H }}
      className={cn(
        'overflow-hidden rounded-md border-2 px-3 py-2 shadow-[2px_3px_0_rgba(0,0,0,0.08)] transition-shadow',
        style.bg,
        style.border,
        // Orphan visual: dashed border + amber tint over the kind color
        // so it's instantly readable as "intentionally disconnected".
        orphan !== undefined && 'border-dashed border-amber-500 bg-amber-50/40',
        // COMMAND_NO_EMITTED_EVENTS: dashed amber border marks the command as
        // pending work: allowed to exist, but visibly unfinished.
        noEmittedEvents && 'border-dashed border-amber-500 bg-amber-50/40',
        // COMMAND_MULTIPLE_ISSUERS: solid red border marks the command as
        // having an ambiguous trigger: error, not pending work.
        multipleIssuers && 'border-red-500 bg-red-50/50',
        // Branch conflict: same red-solid-border treatment as
        // multipleIssuers; both mean "this needs a decision before merge".
        branchStatus === 'conflict' && 'border-red-500 bg-red-50/50',
        selected && 'ring-2 ring-ring shadow-[3px_5px_0_rgba(0,0,0,0.14)]',
      )}
      title={
        multipleIssuers
          ? 'COMMAND_MULTIPLE_ISSUERS: this command is issued by more than one UI or processor; the trigger is ambiguous'
          : noEmittedEvents
            ? 'COMMAND_NO_EMITTED_EVENTS: pending work, no CommandSlice emits events for this command'
            : orphan
              ? `Orphan: ${orphan}`
              : undefined
      }
    >
      {ports.map((p) => (
        <Handle key={p.id} id={p.id} type={p.type} position={p.position} className={handleClass} />
      ))}
      <StickyCardBody
        entity={entity}
        synthetic={synthetic}
        roads={data.roads}
        instance={data.instance}
        moment={data.moment}
        noEmittedEvents={noEmittedEvents}
        multipleIssuers={multipleIssuers}
        branchStatus={branchStatus}
      />
    </div>
  );
}

const SLICE_HEADER_STYLE: Record<string, { bg: string; border: string; label: string }> = {
  commandSlice: { bg: 'bg-sky-50', border: 'border-sky-300', label: 'Command Slice' },
  readModelSlice: { bg: 'bg-emerald-50', border: 'border-emerald-300', label: 'Read Model Slice' },
  automationSlice: { bg: 'bg-violet-50', border: 'border-violet-300', label: 'Automation Slice' },
  uiSlice: { bg: 'bg-zinc-50', border: 'border-zinc-300', label: 'UI Slice' },
};

export function SliceHeaderNode({ data, selected }: NodeProps & { data: SliceHeaderData }) {
  const style = SLICE_HEADER_STYLE[data.sliceKind] ?? {
    bg: 'bg-zinc-50',
    border: 'border-zinc-300',
    label: data.sliceKind,
  };
  // Status comes from tracker overlays, never the slice itself (Decision #25).
  const status = data.status;
  const scenarioCount = Array.isArray(data.entity.raw.scenarios) ? data.entity.raw.scenarios.length : 0;
  return (
    <div
      className={cn(
        'h-full w-full cursor-pointer rounded-md border px-2.5 py-1.5',
        style.bg,
        style.border,
        selected && 'ring-2 ring-ring',
      )}
    >
      <div className="flex items-center gap-1 text-[9px] font-semibold uppercase tracking-wide text-zinc-500">
        {style.label}
        <span className="ml-auto flex items-center gap-1">
          {scenarioCount > 0 ? (
            <span title={`${scenarioCount} scenarios`} className="rounded bg-white/70 px-1 font-mono text-[8px]">
              {scenarioCount} gwt
            </span>
          ) : null}
          {status ? (
            <span
              className={cn(
                'rounded px-1 font-mono text-[8px]',
                status === 'blocked' ? 'bg-red-100 font-bold text-red-700' : 'bg-white/70',
                status === 'done' && 'bg-emerald-100 text-emerald-800',
              )}
            >
              {status}
            </span>
          ) : null}
          {data.staleRefs ? (
            <span
              title={`${data.staleRefs} ref(s) pin an older version; drawer shows which`}
              className="rounded bg-amber-100 px-1 font-mono text-[8px] font-bold text-amber-800"
            >
              v↑{data.staleRefs}
            </span>
          ) : null}
        </span>
      </div>
      <div className="mt-0.5 truncate text-[12px] font-semibold leading-snug text-zinc-800">{data.entity.title}</div>
    </div>
  );
}

export function GapHeaderNode({ data }: NodeProps & { data: GapHeaderData }) {
  return (
    <div className="h-full w-full rounded-md border-2 border-dashed border-zinc-400 bg-zinc-100 px-2.5 py-1.5">
      <div className="flex items-center gap-1 text-[9px] font-bold uppercase tracking-wide text-zinc-600">
        <Info className="h-3 w-3" />
        Implicit projection: no slice yet
      </div>
      <div className="mt-0.5 truncate text-[10px] font-medium leading-snug text-zinc-700">
        {data.readModels.join(', ')} updated by {data.events.join(', ')}
      </div>
    </div>
  );
}

export function EventModelHeaderNode({ data, selected }: NodeProps & { data: StickyData }) {
  return (
    <div
      className={cn(
        'flex h-full w-full cursor-pointer items-center gap-1.5 rounded-md border border-zinc-400 bg-zinc-200/90 px-2.5 text-[11px] font-bold text-zinc-800',
        selected && 'ring-2 ring-ring',
      )}
    >
      <Layers className="h-3 w-3" />
      <span className="truncate">{data.entity.title}</span>
      <span className="ml-auto text-[9px] font-normal uppercase tracking-wide text-zinc-500">event model</span>
    </div>
  );
}

export function StoryboardHeaderNode({
  data,
  selected,
}: NodeProps & { data: StickyData & { waysIn?: { from: string; doc: string }[] } }) {
  const waysIn = data.waysIn ?? [];
  // A header summarizes: just a counter. The per-road data (from + guard)
  // lives in the tooltip and the Inspector.
  return (
    <div
      className={cn(
        'flex h-full w-full cursor-pointer items-center gap-1.5 rounded-md border border-zinc-300 bg-zinc-100/90 px-2.5 text-[11px] font-semibold text-zinc-700',
        selected && 'ring-2 ring-ring',
      )}
    >
      <Clapperboard className="h-3 w-3" />
      <span className="truncate">{data.entity.title}</span>
      {waysIn.length > 0 ? (
        <span
          title={waysIn.map((w) => `from ${w.from}${w.doc ? ` (when ${w.doc})` : ''}`).join('\n')}
          className="inline-flex shrink-0 items-center gap-0.5 rounded border border-amber-300 bg-amber-100 px-1 text-[9px] font-bold text-amber-800"
        >
          <Split aria-hidden className="h-2.5 w-2.5" />
          {waysIn.length}
        </span>
      ) : null}
      <span className="ml-auto shrink-0 text-[9px] font-normal uppercase tracking-wide text-zinc-400">storyboard</span>
    </div>
  );
}

export function BandNode({ width, height }: NodeProps) {
  return <div style={{ width, height }} className="rounded bg-slate-900/[0.04]" />;
}

export function LaneNode({ data, width, height }: NodeProps & { data: LaneData }) {
  return (
    <div
      style={{ width, height, background: data.tint }}
      className={cn(
        'rounded-lg border border-dashed border-zinc-300/80',
        data.entity && 'cursor-pointer',
        data.selected && 'border-solid ring-2 ring-ring',
      )}
    >
      <div
        className={cn(
          'sticky left-0 flex w-fit max-w-[210px] items-start gap-1.5 rounded px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-zinc-500',
          data.selected && 'text-zinc-800',
        )}
      >
        {data.glyph ? <GlyphIcon glyph={data.glyph} className="mt-px h-3 w-3 shrink-0" /> : null}
        <div className="min-w-0">
          <div className="whitespace-nowrap">{data.label}</div>
          {data.entity?.streamId ? (
            <div className="mt-0.5 whitespace-normal break-all font-mono text-[9px] font-normal normal-case tracking-normal text-zinc-400">
              {data.entity.streamId}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

// Band separating bounded contexts when several namespaces are on screen:
// each context renders as its own self-contained group. The band is backed
// by the context's EVENT MODEL entity; click it to open docs, links, and
// annotations in the drawer like any other thing.
export function ContextHeaderNode({
  data,
  selected,
}: NodeProps & { data: { ns: string; title?: string; doc?: string; entity?: StickyData['entity'] } }) {
  return (
    <div
      className={cn(
        'flex h-full w-full items-center gap-2 rounded-md border-2 border-fuchsia-300 bg-fuchsia-50/90 px-3',
        data.entity && 'cursor-pointer',
        selected && 'ring-2 ring-ring',
      )}
    >
      <Globe className="h-3.5 w-3.5 shrink-0 text-fuchsia-600" />
      <span className="shrink-0 text-[12px] font-bold uppercase tracking-wider text-fuchsia-800">{data.ns}</span>
      {data.title ? <span className="min-w-0 truncate text-[11px] text-fuchsia-700">{data.title}</span> : null}
      <span className="ml-auto shrink-0 text-[9px] uppercase tracking-wide text-fuchsia-400">bounded context</span>
    </div>
  );
}

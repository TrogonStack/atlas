import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  type NodeProps,
  type NodeTypes,
  Position,
  ReactFlow,
} from '@xyflow/react';
import { CircleAlert, Globe, Info, TriangleAlert, Waypoints } from 'lucide-react';
import { useMemo } from 'react';
import type { Selection } from '@/components/canvas/Board';
import { useIssues } from '@/components/IssuesProvider';
import { BoardEmptyState } from '@/components/ui/empty-state';
import {
  type BandData,
  buildDomainChart,
  CARD_H,
  CARD_W,
  type ChartMarker,
  type ContextCardData,
  CTX_H,
  CTX_W,
  chartMarkers,
  markerTooltip,
  type SubdomainCardData,
} from '@/lib/domainchart';
import type { Entity, Model } from '@/lib/model';
import { cn } from '@/lib/utils';
import { typedNode } from './nodeTypes';
import { type FocusRequest, useFocusNode } from './useFocusNode';

const handleClass = '!h-1.5 !w-1.5 !border-0 !bg-zinc-400/70';

const TONE = {
  core: { band: 'border-amber-300 bg-amber-50/50', label: 'text-amber-800' },
  supporting: { band: 'border-sky-200 bg-sky-50/40', label: 'text-sky-800' },
  generic: { band: 'border-zinc-200 bg-zinc-50/60', label: 'text-zinc-600' },
  unclassified: { band: 'border-dashed border-zinc-300 bg-zinc-50/40', label: 'text-zinc-500' },
} as const;

const MARKER_STYLE = {
  error: { icon: CircleAlert, tone: 'bg-red-100 text-red-700' },
  warning: { icon: TriangleAlert, tone: 'bg-amber-100 text-amber-800' },
  info: { icon: Info, tone: 'bg-sky-100 text-sky-700' },
} as const;

function FindingMarker({ marker }: { marker?: ChartMarker }) {
  if (!marker) return null;
  const { icon: Icon, tone } = MARKER_STYLE[marker.severity];
  const count = marker.findings.length;
  return (
    <span
      title={markerTooltip(marker)}
      aria-label={`${count} validation ${count === 1 ? 'finding' : 'findings'}: ${marker.findings.map((f) => f.code).join(', ')}`}
      className={cn('ml-auto flex shrink-0 items-center gap-0.5 rounded px-1 py-0.5 text-[9px] font-bold', tone)}
    >
      <Icon aria-hidden="true" className="h-2.5 w-2.5" />
      {count}
    </span>
  );
}

function ClassificationBandNode({ data }: NodeProps & { data: BandData }) {
  const tone = TONE[data.tone];
  return (
    <div className={cn('h-full w-full rounded-xl border-2 px-4 py-2', tone.band)}>
      <span className={cn('text-sm font-bold uppercase tracking-wide', tone.label)}>{data.label}</span>
      <span className="ml-3 text-[11px] text-muted-foreground">{data.hint}</span>
    </div>
  );
}

function DomainHeaderNode({
  data,
}: NodeProps & { data: { entity: Entity; marker?: ChartMarker; selected?: boolean } }) {
  return (
    <div
      className={cn(
        'flex h-full w-full cursor-pointer items-center gap-2 rounded-lg border-2 border-indigo-300 bg-indigo-50 px-4',
        data.selected && 'ring-2 ring-ring',
      )}
    >
      <Globe className="h-4 w-4 shrink-0 text-indigo-600" />
      <span className="whitespace-nowrap text-sm font-bold text-indigo-900">{data.entity.title}</span>
      <span className="min-w-0 truncate text-[11px] text-indigo-700/70">{data.entity.doc}</span>
      <FindingMarker marker={data.marker} />
    </div>
  );
}

function SubdomainCardNode({ data }: NodeProps & { data: SubdomainCardData }) {
  return (
    <div
      style={{ width: CARD_W, height: CARD_H }}
      className={cn(
        'cursor-pointer overflow-hidden rounded-lg border-2 border-indigo-200 bg-white px-3 py-2 shadow-[3px_4px_0_rgba(0,0,0,0.06)]',
        data.selected && 'ring-2 ring-ring',
      )}
    >
      <Handle id="b" type="target" position={Position.Bottom} className={handleClass} />
      <div className="flex items-center gap-2">
        <span className="truncate text-xs font-semibold">{data.entity.title}</span>
        <FindingMarker marker={data.marker} />
        {data.classification ? (
          <span
            className={cn(
              'shrink-0 rounded px-1.5 py-0.5 text-[9px] font-bold uppercase',
              !data.marker && 'ml-auto',
              data.classification === 'core'
                ? 'bg-amber-100 text-amber-800'
                : data.classification === 'generic'
                  ? 'bg-zinc-100 text-zinc-600'
                  : 'bg-sky-100 text-sky-700',
            )}
          >
            {data.classification}
          </span>
        ) : null}
      </div>
      <p className="mt-1 line-clamp-3 text-[10px] leading-snug text-muted-foreground">{data.entity.doc}</p>
    </div>
  );
}

function ContextCardNode({ data }: NodeProps & { data: ContextCardData }) {
  const others = data.alsoRealizes.map((o) => o.title).join(', ');
  return (
    <div
      style={{ width: CTX_W, height: CTX_H }}
      className={cn(
        'flex cursor-pointer flex-col gap-0.5 overflow-hidden rounded-lg border-2 border-fuchsia-300 bg-fuchsia-50 px-3 py-1.5',
        data.selected && 'ring-2 ring-ring',
      )}
    >
      <Handle id="t" type="source" position={Position.Top} className={handleClass} />
      <div className="flex items-center gap-1.5">
        <span className="truncate text-xs font-semibold text-fuchsia-900">{data.entity.title}</span>
        <FindingMarker marker={data.marker} />
      </div>
      <div className="truncate font-mono text-[10px] text-fuchsia-700/70">{data.entity.id.namespace}</div>
      {data.splitCandidate ? (
        <div
          title={`${data.splitCandidate}: one boundary carries several investment levels, also realizes ${others}`}
          className="max-w-full self-start truncate rounded bg-orange-100 px-1.5 py-0.5 text-[9px] font-bold text-orange-800"
        >
          {data.splitCandidate}
        </div>
      ) : others ? (
        <div title={`Also realizes ${others}`} className="truncate text-[10px] text-fuchsia-800">
          also realizes {others}
        </div>
      ) : null}
    </div>
  );
}

const nodeTypes: NodeTypes = {
  classificationBand: typedNode(ClassificationBandNode),
  domainHeader: typedNode(DomainHeaderNode),
  subdomainCard: typedNode(SubdomainCardNode),
  contextCard: typedNode(ContextCardNode),
};

export function DomainChartBoard({
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
  const issues = useIssues();
  const markers = useMemo(() => chartMarkers(issues.all), [issues]);
  const { nodes, edges } = useMemo(() => buildDomainChart(model, markers), [model, markers]);
  useFocusNode(focus, nodes, 'subdomainCard');

  const displayNodes = useMemo(() => {
    const keys = new Set(
      selections.filter((s): s is Extract<Selection, { type: 'entity' }> => s.type === 'entity').map((s) => s.key),
    );
    return nodes.map((n) => {
      const entity = (n.data as { entity?: { key: string } }).entity;
      if (!entity) return n;
      return { ...n, data: { ...n.data, selected: keys.has(entity.key) } };
    });
  }, [nodes, selections]);

  const hasKnowledge = nodes.some((n) => n.type === 'subdomainCard' || n.type === 'domainHeader');

  return (
    <div className="relative h-full w-full">
      <ReactFlow
        nodes={displayNodes}
        edges={edges}
        nodeTypes={nodeTypes}
        fitView
        minZoom={0.05}
        proOptions={{ hideAttribution: true }}
        onNodeClick={(e, node) => {
          const entity = (node.data as { entity?: { key: string } }).entity;
          if (!entity) return;
          onSelect(entity.key, undefined, e.metaKey || e.ctrlKey || e.shiftKey);
        }}
        onPaneClick={() => onSelect(undefined)}
        nodesConnectable={false}
        nodesDraggable={false}
        elevateNodesOnSelect={false}
      >
        <Background variant={BackgroundVariant.Dots} gap={18} size={1.2} color="#d4d4d8" />
        <Controls showInteractive={false} />
      </ReactFlow>
      {!hasKnowledge ? (
        <BoardEmptyState
          icon={Waypoints}
          title="No domains mapped yet"
          hint={
            <>
              Declare <span className="font-mono">Domain</span> and <span className="font-mono">Subdomain</span>{' '}
              manifests and apply them with <span className="font-mono">trogon-atlas apply</span>. Studio only reads the
              model.
            </>
          }
        >
          This chart shows how the business problem is split up: the domain, its subdomains, and which bounded contexts
          take care of each one.
        </BoardEmptyState>
      ) : null}
      <div className="absolute bottom-3 right-3 flex flex-col gap-1 rounded-md border border-border bg-card/95 px-2.5 py-2 text-[10px]">
        <div className="flex items-center gap-1.5">
          <span className="inline-block h-0.5 w-4 bg-indigo-400" />
          where investment goes: subdomains by classification, contexts realizing them
        </div>
        <div className="flex items-center gap-1.5">
          <span className="inline-block h-2.5 w-4 rounded-sm border border-fuchsia-300 bg-fuchsia-50" />a context
          realizing several subdomains appears under each one
        </div>
      </div>
    </div>
  );
}

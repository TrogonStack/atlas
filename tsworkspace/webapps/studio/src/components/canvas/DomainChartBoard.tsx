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
import { Globe } from 'lucide-react';
import { useMemo } from 'react';
import type { Selection } from '@/components/canvas/Board';
import {
  type BandData,
  buildDomainChart,
  CARD_H,
  CARD_W,
  type ContextCardData,
  CTX_H,
  CTX_W,
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

function ClassificationBandNode({ data }: NodeProps & { data: BandData }) {
  const tone = TONE[data.tone];
  return (
    <div className={cn('h-full w-full rounded-xl border-2 px-4 py-2', tone.band)}>
      <span className={cn('text-sm font-bold uppercase tracking-wide', tone.label)}>{data.label}</span>
      <span className="ml-3 text-[11px] text-muted-foreground">{data.hint}</span>
    </div>
  );
}

function DomainHeaderNode({ data }: NodeProps & { data: { entity: Entity; selected?: boolean } }) {
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
        {data.classification ? (
          <span
            className={cn(
              'ml-auto shrink-0 rounded px-1.5 py-0.5 text-[9px] font-bold uppercase',
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
  return (
    <div
      style={{ width: CTX_W, height: CTX_H }}
      className={cn(
        'cursor-pointer overflow-hidden rounded-lg border-2 border-fuchsia-300 bg-fuchsia-50 px-3 py-1.5',
        data.selected && 'ring-2 ring-ring',
      )}
    >
      <Handle id="t" type="source" position={Position.Top} className={handleClass} />
      <div className="text-xs font-semibold text-fuchsia-900">{data.entity.title}</div>
      <div className="font-mono text-[10px] text-fuchsia-700/70">{data.entity.id.namespace}</div>
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
  const { nodes, edges } = useMemo(() => buildDomainChart(model), [model]);
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
        <div className="absolute inset-0 flex items-center justify-center">
          <div className="max-w-md rounded-lg border border-border bg-card px-6 py-4 text-sm text-muted-foreground">
            No domains or subdomains in the store yet. Add a Domain root entity with
            <span className="font-mono"> id.slug == id.namespace</span> and the Subdomains that partition its problem
            space (Decision #29).
          </div>
        </div>
      ) : null}
      <div className="absolute bottom-3 right-3 rounded-md border border-border bg-card/95 px-2.5 py-2 text-[10px]">
        <div className="flex items-center gap-1.5">
          <span className="inline-block h-0.5 w-4 bg-indigo-400" />
          where investment goes: subdomains by classification, contexts realizing them
        </div>
      </div>
    </div>
  );
}

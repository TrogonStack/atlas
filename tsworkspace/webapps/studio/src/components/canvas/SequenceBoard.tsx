import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  type NodeProps,
  type NodeTypes,
  Position,
  ReactFlow,
  useReactFlow,
} from '@xyflow/react';
import { Repeat } from 'lucide-react';
import { useQueryState } from 'nuqs';
import { useMemo } from 'react';
import type { SelectedEdge, Selection } from '@/components/canvas/Board';
import { BoardTopRight } from '@/components/canvas/BoardTopRight';
import {
  ContextHeaderNode,
  kindStyle,
  SliceHeaderNode,
  StickyCardBody,
  StoryboardHeaderNode,
} from '@/components/canvas/StickyNode';
import type { BranchStatus } from '@/lib/branch';
import { type BoardEdgeData, NODE_H, NODE_W } from '@/lib/layout';
import type { Entity, Model } from '@/lib/model';
import { slugKey } from '@/lib/model';
import { buildSequences } from '@/lib/multiboard';
import type { SeqStickyData } from '@/lib/sequence';
import { cn } from '@/lib/utils';
import { typedNode } from './nodeTypes';
import { type FocusRequest, useFocusNode } from './useFocusNode';

const handleClass = '!h-1.5 !w-1.5 !border-0 !bg-zinc-400/70';

function SeqStickyNode({ data }: NodeProps & { data: SeqStickyData }) {
  const style = kindStyle(data.entity.kind);
  return (
    <div
      style={{ width: NODE_W, height: NODE_H }}
      className={cn(
        'cursor-pointer overflow-hidden rounded-md border-2 px-3 py-2 shadow-[2px_3px_0_rgba(0,0,0,0.08)] transition-shadow',
        style.bg,
        style.border,
        data.selected && 'ring-2 ring-ring shadow-[3px_5px_0_rgba(0,0,0,0.14)]',
      )}
    >
      <Handle id="l" type="target" position={Position.Left} className={handleClass} />
      <Handle id="r" type="source" position={Position.Right} className={handleClass} />
      <StickyCardBody
        entity={data.entity}
        instance={data.instance}
        moment={data.moment}
        branchStatus={data.branchStatus}
      />
    </div>
  );
}

// Time only moves forward: a lifecycle cycle ends in this ghost card (a new
// moment re-entering an earlier storyboard) instead of a backward edge.
function SeqLoopStubNode({ data }: NodeProps & { data: { entity?: Entity; doc: string } }) {
  return (
    <div
      style={{ width: NODE_W, height: NODE_H }}
      className="cursor-pointer overflow-hidden rounded-md border-2 border-dashed border-amber-400 bg-amber-50 px-3 py-2"
    >
      <Handle id="l" type="target" position={Position.Left} className={handleClass} />
      <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-amber-700">
        <Repeat className="h-3 w-3" />
        Road continues
      </div>
      <div className="mt-1 text-[13px] font-semibold leading-snug text-zinc-900">{data.entity?.title}</div>
      {data.doc ? <div className="mt-1 text-[10px] leading-tight text-zinc-600">when {data.doc}</div> : null}
      <div className="mt-1 text-[9px] font-medium text-amber-700">a new moment: click to jump ↗</div>
    </div>
  );
}

const nodeTypes: NodeTypes = {
  seqSticky: typedNode(SeqStickyNode),
  seqSliceHeader: typedNode(SliceHeaderNode),
  seqStoryboardHeader: typedNode(StoryboardHeaderNode),
  seqLoopStub: typedNode(SeqLoopStubNode),
  contextHeader: typedNode(ContextHeaderNode),
};

export function SequenceBoard({
  model,
  selections,
  onSelect,
  onSelectEdge,
  focus,
  branchStatusLookup,
}: {
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
  /** BADGES (Phase 3: Studio): same post-processing convention as Board.tsx. */
  branchStatusLookup?: Map<string, BranchStatus>;
}) {
  const { nodes: builtNodes, edges } = useMemo(() => buildSequences(model), [model]);
  const nodes = useMemo(() => {
    if (!branchStatusLookup || branchStatusLookup.size === 0) return builtNodes;
    return builtNodes.map((n) => {
      if (n.type !== 'seqSticky') return n;
      const entity = (n.data as SeqStickyData).entity;
      const branchStatus = branchStatusLookup.get(slugKey(entity.kind, entity.id));
      return branchStatus ? { ...n, data: { ...n.data, branchStatus } } : n;
    });
  }, [builtNodes, branchStatusLookup]);
  useFocusNode(focus, nodes, 'seqSticky');
  const { setCenter, getZoom } = useReactFlow();
  const keys = useMemo(() => new Set(selections.filter((s) => s.type === 'entity').map((s) => s.key)), [selections]);

  // Persists as ?rm=<key> so a refresh keeps the filter.
  const [readModelFilter, setReadModelFilter] = useQueryState('rm');

  const readModelOptions = useMemo(() => {
    const seen = new Map<string, string>();
    for (const n of nodes) {
      if (n.type !== 'seqSticky') continue;
      const entity = (n.data as { entity?: Entity }).entity;
      if (!entity || entity.kind !== 'readModel') continue;
      if (!seen.has(entity.key)) seen.set(entity.key, entity.title ?? entity.key);
    }
    return Array.from(seen.entries()).map(([key, label]) => ({ key, label }));
  }, [nodes]);

  const readModelLitIds = useMemo(() => {
    if (!readModelFilter) return null;
    const occurrenceIds = new Set<string>();
    for (const n of nodes) {
      if (n.type !== 'seqSticky') continue;
      const entity = (n.data as { entity?: Entity }).entity;
      if (entity?.kind === 'readModel' && entity.key === readModelFilter) occurrenceIds.add(n.id);
    }
    const predecessorIds = new Set<string>();
    for (const e of edges) {
      if (occurrenceIds.has(e.target)) predecessorIds.add(e.source);
    }
    const litNodeIds = new Set([...occurrenceIds, ...predecessorIds]);
    const litEdgeIds = new Set<string>();
    for (const e of edges) {
      if (occurrenceIds.has(e.target)) litEdgeIds.add(e.id);
    }
    return { litNodeIds, litEdgeIds };
  }, [nodes, edges, readModelFilter]);

  // Sticky selection focus set, same contract as Board.focusSets: selecting
  // a sticky dims unrelated stickies and connections.
  const focusSets = useMemo(() => {
    const entitySel = selections.filter((s): s is Extract<Selection, { type: 'entity' }> => s.type === 'entity');
    const selectedEdgeIds = new Set(selections.filter((s) => s.type === 'edge').map((s) => s.id));
    const occIds = new Set(entitySel.filter((s) => s.occId).map((s) => s.occId));
    const keysWithoutOcc = new Set(entitySel.filter((s) => !s.occId).map((s) => s.key));
    const stickyIds = new Set<string>();
    for (const n of nodes) {
      if (n.type !== 'seqSticky') continue;
      if (occIds.has(n.id) || keysWithoutOcc.has((n.data as SeqStickyData).entity.key)) stickyIds.add(n.id);
    }
    if (stickyIds.size === 0 && selectedEdgeIds.size === 0) return undefined;
    const litEdgeIds = new Set(selectedEdgeIds);
    const litNodeIds = new Set(stickyIds);
    for (const e of edges) {
      if (stickyIds.has(e.source) || stickyIds.has(e.target)) litEdgeIds.add(e.id);
      if (litEdgeIds.has(e.id)) {
        litNodeIds.add(e.source);
        litNodeIds.add(e.target);
      }
    }
    return { selectedEdgeIds, litEdgeIds, litNodeIds, dimStickies: stickyIds.size > 0 };
  }, [nodes, edges, selections]);

  // Clicking a connection emphasizes it and fades the rest, same as Board.
  const displayEdges = useMemo(() => {
    if (!focusSets && !readModelLitIds) return edges;
    return edges.map((e) => {
      let next = e;
      if (focusSets) {
        if (!focusSets.litEdgeIds.has(e.id)) {
          next = { ...next, style: { ...next.style, opacity: 0.15 } };
        } else if (focusSets.selectedEdgeIds.has(e.id)) {
          next = { ...next, style: { ...next.style, strokeWidth: 3.5 }, zIndex: 20 };
        } else if (focusSets.dimStickies || focusSets.selectedEdgeIds.size > 0) {
          next = { ...next, style: { ...next.style, strokeWidth: 2.5 }, zIndex: 20 };
        }
      }
      if (readModelLitIds && !readModelLitIds.litEdgeIds.has(e.id)) {
        return { ...next, style: { ...next.style, opacity: 0.12 } };
      }
      return next;
    });
  }, [edges, focusSets, readModelLitIds]);

  // Sequence cards are temporal occurrences: canvas clicks pin the moment
  // (occId = node id); drawer/sidebar selections highlight the type.
  const displayNodes = useMemo(() => {
    const entitySel = selections.filter((s): s is Extract<Selection, { type: 'entity' }> => s.type === 'entity');
    const occIds = new Set(entitySel.filter((s) => s.occId).map((s) => s.occId));
    const keysWithoutOcc = new Set(entitySel.filter((s) => !s.occId).map((s) => s.key));
    const dimStickies = focusSets?.dimStickies ?? false;
    return nodes.map((n) => {
      const entity = (n.data as { entity?: { key: string } }).entity;
      if (!entity) return n;
      const filterDimmed = readModelLitIds && !readModelLitIds.litNodeIds.has(n.id);
      if (n.type === 'seqSticky') {
        const selected = occIds.has(n.id) || keysWithoutOcc.has(entity.key);
        const focusDimmed = dimStickies && !focusSets?.litNodeIds.has(n.id);
        const base = { ...n, data: { ...n.data, selected } };
        if (focusDimmed) return { ...base, style: { ...base.style, opacity: 0.15 } };
        return filterDimmed ? { ...base, style: { ...base.style, opacity: 0.12 } } : base;
      }
      const base = { ...n, selected: keys.has(entity.key) };
      return filterDimmed ? { ...base, style: { ...base.style, opacity: 0.12 } } : base;
    });
  }, [nodes, selections, keys, readModelLitIds, focusSets]);

  return (
    <div className="relative h-full w-full">
      {readModelOptions.length > 0 && (
        <BoardTopRight>
          <div className="rounded-md border border-border bg-card/95 px-2.5 py-2 text-[10px] shadow-sm">
            <select
              value={readModelFilter ?? ''}
              onChange={(e) => setReadModelFilter(e.target.value || null)}
              className="bg-transparent text-[11px] outline-none cursor-pointer"
            >
              <option value="">All read models</option>
              {readModelOptions.map((opt) => (
                <option key={opt.key} value={opt.key}>
                  {opt.label}
                </option>
              ))}
            </select>
          </div>
        </BoardTopRight>
      )}
      <ReactFlow
        nodes={displayNodes}
        edges={displayEdges}
        nodeTypes={nodeTypes}
        fitView
        minZoom={0.05}
        proOptions={{ hideAttribution: true }}
        onEdgeClick={(e, edge) => {
          const additive = e.metaKey || e.ctrlKey || e.shiftKey;
          onSelectEdge({ id: edge.id, data: edge.data as BoardEdgeData }, additive);
        }}
        onNodeClick={(e, node) => {
          const entity = (node.data as { entity?: { key: string } }).entity;
          if (!entity) return;
          if (node.type === 'seqLoopStub') {
            // The stub IS the jump: pan to the storyboard the road re-enters.
            const target = nodes.find((n) => n.id.endsWith(`seq-sb:${entity.key}`));
            if (target) {
              const w = Number((target.style as { width?: number } | undefined)?.width ?? NODE_W) || NODE_W;
              setCenter(target.position.x + w / 2, target.position.y + 140, {
                duration: 600,
                zoom: Math.max(getZoom(), 0.6),
              });
            }
            onSelect(entity.key);
            return;
          }
          const additive = e.metaKey || e.ctrlKey || e.shiftKey;
          const d = node.data as { instanceUid?: string; moment?: { key?: string } };
          onSelect(entity.key, node.type === 'seqSticky' ? node.id : undefined, additive, d.instanceUid, d.moment?.key);
        }}
        onPaneClick={() => onSelect(undefined)}
        nodesConnectable={false}
        nodesDraggable={false}
        elevateNodesOnSelect={false}
      >
        <Background variant={BackgroundVariant.Dots} gap={18} size={1.2} color="#d4d4d8" />
        <Controls showInteractive={false} />
      </ReactFlow>
      <div className="absolute bottom-3 right-3 rounded-md border border-border bg-card/95 px-2.5 py-2 text-[10px]">
        <div className="flex items-center gap-1.5">
          <span className="inline-block h-0.5 w-4 bg-zinc-400" />
          happens next, time only moves forward; cycles end in a ⟳ stub
        </div>
      </div>
    </div>
  );
}

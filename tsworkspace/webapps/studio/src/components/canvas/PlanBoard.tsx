import {
  applyNodeChanges,
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  type Node,
  type NodeChange,
  type NodeProps,
  type NodeTypes,
  Position,
  ReactFlow,
} from '@xyflow/react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { Selection } from '@/components/canvas/Board';
import { kindStyle } from '@/components/canvas/StickyNode';
import type { Model } from '@/lib/model';
import { buildPlan, PLAN_NODE_W, type PlanArtifactData, type PlanPhaseData } from '@/lib/plan';
import { cn } from '@/lib/utils';
import { typedNode } from './nodeTypes';
import { type FocusRequest, useFocusNode } from './useFocusNode';

function PlanArtifactNode({ data, selected }: NodeProps & { data: PlanArtifactData }) {
  const style = kindStyle(data.entity.kind);
  const Icon = style.icon;
  return (
    <div
      style={{ width: PLAN_NODE_W }}
      className={cn(
        'cursor-pointer rounded-md border-2 px-2.5 py-1.5 shadow-[1px_2px_0_rgba(0,0,0,0.06)]',
        style.bg,
        style.border,
        selected && 'ring-2 ring-ring',
      )}
    >
      <Handle id="t" type="target" position={Position.Top} className="!h-1.5 !w-1.5 !border-0 !bg-zinc-400/70" />
      <Handle id="b" type="source" position={Position.Bottom} className="!h-1.5 !w-1.5 !border-0 !bg-zinc-400/70" />
      <div className="flex items-center gap-1 text-[8px] font-semibold uppercase tracking-wide text-zinc-600">
        <Icon className="h-2.5 w-2.5" />
        {style.label}
        <span className="ml-auto flex items-center gap-1 font-mono">
          {data.sliceCount > 0 ? (
            <span title={`specified by ${data.sliceCount} slice(s)`} className="rounded bg-white/80 px-1">
              {data.sliceCount} slc
            </span>
          ) : null}
          {data.scenarioCount > 0 ? (
            <span title={`${data.scenarioCount} scenarios`} className="rounded bg-white/80 px-1">
              {data.scenarioCount} gwt
            </span>
          ) : null}
          {data.status ? (
            <span
              className={cn(
                'rounded px-1',
                data.status === 'blocked' ? 'bg-red-100 font-bold text-red-700' : 'bg-white/80',
                data.status === 'done' && 'bg-emerald-100 text-emerald-800',
              )}
            >
              {data.status}
            </span>
          ) : null}
        </span>
      </div>
      <div className="mt-0.5 truncate text-[11px] font-semibold leading-snug text-zinc-900">{data.entity.title}</div>
    </div>
  );
}

function PlanPhaseNode({ data }: NodeProps & { data: PlanPhaseData }) {
  return (
    <div className="flex h-full w-full items-center gap-2 rounded-md border border-dashed border-zinc-400 bg-zinc-100/90 px-3">
      <span className="text-[12px] font-bold text-zinc-700">Phase {data.phase}</span>
      <span className="text-[10px] text-zinc-500">
        {data.items > 1 ? `${data.items} deliverables in parallel` : '1 deliverable'}
      </span>
    </div>
  );
}

const nodeTypes: NodeTypes = {
  planArtifact: typedNode(PlanArtifactNode),
  planPhase: typedNode(PlanPhaseNode),
};

export function PlanBoard({
  model,
  selections,
  onSelect,
  focus: focusRequest,
}: {
  model: Model;
  selections: Selection[];
  onSelect: (key: string | undefined, occId?: string, additive?: boolean) => void;
  focus?: FocusRequest;
}) {
  const { nodes, edges } = useMemo(() => buildPlan(model), [model]);
  useFocusNode(focusRequest, nodes, 'planArtifact');
  const keys = useMemo(() => new Set(selections.filter((s) => s.type === 'entity').map((s) => s.key)), [selections]);
  const [focusedEdgeId, setFocusedEdgeId] = useState<string>();

  // Selecting a deliverable (or a connection) focuses the full upstream
  // closure: everything it transitively depends on stays visible; the rest
  // fades.
  const focus = useMemo(() => {
    const roots = [...keys].map((k) => `plan:${k}`).filter((id) => nodes.some((n) => n.id === id));
    const focusedEdge = focusedEdgeId ? edges.find((e) => e.id === focusedEdgeId) : undefined;
    if (focusedEdge) roots.push(focusedEdge.target);
    if (roots.length === 0) return null;
    const preds = new Map<string, string[]>();
    for (const e of edges) {
      const list = preds.get(e.target) ?? [];
      list.push(e.source);
      preds.set(e.target, list);
    }
    const keep = new Set<string>();
    const stack = roots;
    while (stack.length > 0) {
      const id = stack.pop();
      if (!id || keep.has(id)) continue;
      keep.add(id);
      for (const p of preds.get(id) ?? []) stack.push(p);
    }
    return keep;
  }, [keys, nodes, edges, focusedEdgeId]);

  const displayNodes = useMemo(
    () =>
      nodes.map((n) => {
        if (n.type !== 'planArtifact') return n;
        const selected = keys.has((n.data as PlanArtifactData).entity.key);
        const faded = focus !== null && !focus.has(n.id);
        return { ...n, selected, style: faded ? { ...n.style, opacity: 0.12 } : { ...n.style, opacity: 1 } };
      }),
    [nodes, keys, focus],
  );
  const displayEdges = useMemo(() => {
    if (!focus) return edges;
    return edges.map((e) =>
      focus.has(e.source) && focus.has(e.target)
        ? {
            ...e,
            style: { stroke: '#52525b', strokeWidth: e.id === focusedEdgeId ? 3 : 2.2 },
            zIndex: 10,
          }
        : { ...e, style: { ...e.style, opacity: 0.05 } },
    );
  }, [edges, focus, focusedEdgeId]);

  const [rfNodes, setRfNodes] = useState(displayNodes);
  useEffect(() => {
    setRfNodes(displayNodes);
  }, [displayNodes]);
  const onNodesChange = useCallback((changes: NodeChange<Node>[]) => {
    setRfNodes((nds) => applyNodeChanges(changes, nds));
  }, []);

  return (
    <div className="relative h-full w-full">
      <ReactFlow
        nodes={rfNodes}
        edges={displayEdges}
        nodeTypes={nodeTypes}
        fitView
        minZoom={0.1}
        proOptions={{ hideAttribution: true }}
        onNodesChange={onNodesChange}
        onNodeClick={(e, node) => {
          if (node.type !== 'planArtifact') return;
          setFocusedEdgeId(undefined);
          const additive = e.metaKey || e.ctrlKey || e.shiftKey;
          onSelect((node.data as PlanArtifactData).entity.key, undefined, additive);
        }}
        onEdgeClick={(_, edge) => {
          onSelect(undefined);
          setFocusedEdgeId((cur) => (cur === edge.id ? undefined : edge.id));
        }}
        onPaneClick={() => {
          setFocusedEdgeId(undefined);
          onSelect(undefined);
        }}
        nodesConnectable={false}
        nodesDraggable={false}
        elevateNodesOnSelect
      >
        <Background variant={BackgroundVariant.Dots} gap={18} size={1.2} color="#d4d4d8" />
        <Controls showInteractive={false} />
      </ReactFlow>
      <div className="absolute bottom-3 right-3 rounded-md border border-border bg-card/95 px-2.5 py-2 text-[10px]">
        <div className="flex items-center gap-1.5">
          <span className="inline-block h-0.5 w-4 bg-zinc-400" />
          contract required by · click anything to trace its chain
        </div>
      </div>
    </div>
  );
}

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
import { GitBranch } from 'lucide-react';
import { useMemo } from 'react';
import type { Selection } from '@/components/canvas/Board';
import { buildContextMap, CONTEXT_MAP_H, CONTEXT_MAP_W, type ContextMapNodeData } from '@/lib/contextmap';
import type { Model } from '@/lib/model';
import { cn } from '@/lib/utils';
import { typedNode } from './nodeTypes';

const handleClass = '!h-1.5 !w-1.5 !border-0 !bg-fuchsia-400/80';

function ContextMapNode({ data }: NodeProps & { data: ContextMapNodeData }) {
  return (
    <div
      style={{ width: CONTEXT_MAP_W, height: CONTEXT_MAP_H }}
      className={cn(
        'cursor-pointer overflow-hidden rounded-lg border-2 border-fuchsia-300 bg-fuchsia-50 px-3 py-2 shadow-[3px_4px_0_rgba(0,0,0,0.06)]',
        data.selected && 'ring-2 ring-ring',
      )}
    >
      <Handle id="in" type="target" position={Position.Left} className={handleClass} />
      <Handle id="out" type="source" position={Position.Right} className={handleClass} />
      <div className="flex items-center gap-2">
        <GitBranch className="h-3.5 w-3.5 shrink-0 text-fuchsia-700" />
        <span className="min-w-0 truncate text-xs font-semibold text-fuchsia-950">{data.entity.title}</span>
      </div>
      <div className="mt-1 truncate font-mono text-[10px] text-fuchsia-700/75">{data.entity.id.namespace}</div>
      <div className="mt-1.5 flex gap-1.5 text-[9px] font-semibold uppercase text-fuchsia-800">
        <span className="rounded bg-white/70 px-1.5 py-0.5">in {data.inbound}</span>
        <span className="rounded bg-white/70 px-1.5 py-0.5">out {data.outbound}</span>
      </div>
    </div>
  );
}

const nodeTypes: NodeTypes = {
  contextMapNode: typedNode(ContextMapNode),
};

export function ContextMapBoard({
  model,
  selections,
  onSelect,
}: {
  model: Model;
  selections: Selection[];
  onSelect: (key: string | undefined, occId?: string, additive?: boolean) => void;
}) {
  const { nodes, edges } = useMemo(() => buildContextMap(model), [model]);
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
      {nodes.length === 0 ? (
        <div className="absolute inset-0 flex items-center justify-center">
          <div className="max-w-md rounded-lg border border-border bg-card px-6 py-4 text-sm text-muted-foreground">
            No bounded contexts in the Overview data yet.
          </div>
        </div>
      ) : null}
      <div className="absolute bottom-3 right-3 rounded-md border border-border bg-card/95 px-2.5 py-2 text-[10px]">
        <div className="flex items-center gap-1.5">
          <span className="inline-block h-0.5 w-4 bg-fuchsia-400" />
          published events into subscription views
        </div>
      </div>
    </div>
  );
}

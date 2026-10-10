import {
  applyNodeChanges,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  type Node,
  type NodeChange,
  type NodeTypes,
  ReactFlow,
  useReactFlow,
} from '@xyflow/react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { BranchStatus } from '@/lib/branch';
import type { BoardEdgeData, LaneData, StickyData } from '@/lib/layout';
import type { Model } from '@/lib/model';
import { slugKey } from '@/lib/model';
import { layoutBoards } from '@/lib/multiboard';
import { typedNode } from './nodeTypes';
import {
  BandNode,
  ContextHeaderNode,
  EventModelHeaderNode,
  GapHeaderNode,
  LaneNode,
  SliceHeaderNode,
  StickyNode,
  StoryboardHeaderNode,
} from './StickyNode';
import { type FocusRequest, useFocusNode } from './useFocusNode';
import { usePersistedViewport } from './usePersistedViewport';

const nodeTypes: NodeTypes = {
  sticky: typedNode(StickyNode),
  lane: typedNode(LaneNode),
  sliceHeader: typedNode(SliceHeaderNode),
  storyboardHeader: typedNode(StoryboardHeaderNode),
  eventModelHeader: typedNode(EventModelHeaderNode),
  gapHeader: typedNode(GapHeaderNode),
  band: typedNode(BandNode),
  contextHeader: typedNode(ContextHeaderNode),
};

const MINIMAP_COLORS: Record<string, string> = {
  event: '#fb923c',
  command: '#38bdf8',
  readModel: '#34d399',
  processor: '#a78bfa',
  ui: '#e4e4e7',
  persona: '#fde68a',
  screen: '#99f6e4',
  term: '#d9f99d',
  ambiguity: '#fecdd3',
  schema: '#e2e8f0',
  project: '#cffafe',
};

export interface SelectedEdge {
  id: string;
  data: BoardEdgeData;
}

// One unified selection list: entities (by key, optionally pinned to a board
// occurrence) and connections, mixable via modifier-click.
export type Selection =
  | { type: 'entity'; key: string; occId?: string; instanceUid?: string; momentKey?: string }
  | { type: 'edge'; id: string; data: BoardEdgeData };

export function Board({
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
  /** BADGES (Phase 3: Studio): stamped onto matching `sticky` nodes below the layout memo. */
  branchStatusLookup?: Map<string, BranchStatus>;
}) {
  const { nodes: layoutNodes, edges } = useMemo(() => layoutBoards(model), [model]);
  // Post-processing step, deliberately not threaded through layoutBoards
  // itself: `composeGroups` (in multiboard.ts) always wraps the layout
  // engine, so extending its signature would ripple through every board
  // kind. Matching is by (kind, namespace, slug), version-agnostic, same
  // as multiboard's own `findPlaced` seam-stitching precedent.
  const nodes = useMemo(() => {
    if (!branchStatusLookup || branchStatusLookup.size === 0) return layoutNodes;
    return layoutNodes.map((n) => {
      if (n.type !== 'sticky') return n;
      const entity = (n.data as StickyData).entity;
      const branchStatus = branchStatusLookup.get(slugKey(entity.kind, entity.id));
      return branchStatus ? { ...n, data: { ...n.data, branchStatus } } : n;
    });
  }, [layoutNodes, branchStatusLookup]);
  const { fitView } = useReactFlow();
  useFocusNode(focus, nodes, 'sticky');
  const viewportKey = `${typeof window === 'undefined' ? 'board' : window.location.pathname}:board`;
  const { onMoveEnd, restored } = usePersistedViewport(viewportKey, !focus);

  // Focus set for the selection: explicitly selected connections plus every
  // connection touching a selected sticky, and the stickies on either end.
  // Selecting a sticky dims unrelated stickies and connections the same way
  // selecting a connection already dims unrelated connections, so the flow
  // in and out of the selection reads at a glance.
  const focusSets = useMemo(() => {
    const entitySel = selections.filter((s): s is Extract<Selection, { type: 'entity' }> => s.type === 'entity');
    const selectedEdgeIds = new Set(selections.filter((s) => s.type === 'edge').map((s) => s.id));
    const occIds = new Set(entitySel.filter((s) => s.occId).map((s) => s.occId));
    const keysWithoutOcc = new Set(entitySel.filter((s) => !s.occId).map((s) => s.key));
    const stickyIds = new Set<string>();
    for (const n of nodes) {
      if (n.type !== 'sticky') continue;
      if (occIds.has(n.id) || keysWithoutOcc.has((n.data as StickyData).entity.key)) stickyIds.add(n.id);
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
    // Stickies only dim for sticky selections: a connection-only selection
    // keeps the historical edges-fade-nodes-stay behavior.
    return { selectedEdgeIds, litEdgeIds, litNodeIds, dimStickies: stickyIds.size > 0 };
  }, [nodes, edges, selections]);

  const displayEdges = useMemo(() => {
    if (!focusSets) return edges;
    return edges.map((e) => {
      if (!focusSets.litEdgeIds.has(e.id)) return { ...e, style: { ...e.style, opacity: 0.15 } };
      const emphasized = focusSets.selectedEdgeIds.has(e.id);
      return { ...e, style: { ...e.style, strokeWidth: emphasized ? 3.5 : 2.5 }, zIndex: 20 };
    });
  }, [edges, focusSets]);

  // Node ids are temporal occurrences (`entityKey@col`); only the clicked
  // occurrence highlights; its twins are different moments in time. Slice
  // and storyboard headers are one-per-entity, so they highlight by key.
  const displayNodes = useMemo(() => {
    const entitySel = selections.filter((s): s is Extract<Selection, { type: 'entity' }> => s.type === 'entity');
    const keys = new Set(entitySel.map((s) => s.key));
    const occIds = new Set(entitySel.filter((s) => s.occId).map((s) => s.occId));
    const keysWithoutOcc = new Set(entitySel.filter((s) => !s.occId).map((s) => s.key));
    const dimStickies = focusSets?.dimStickies ?? false;
    return nodes.map((n) => {
      if (n.type === 'sticky') {
        const entityKey = (n.data as StickyData).entity.key;
        const selected = occIds.has(n.id) || keysWithoutOcc.has(entityKey);
        const dimmed = dimStickies && !focusSets?.litNodeIds.has(n.id);
        return dimmed ? { ...n, selected, style: { ...n.style, opacity: 0.15 } } : { ...n, selected };
      }
      if (n.type === 'sliceHeader' || n.type === 'storyboardHeader' || n.type === 'eventModelHeader')
        return { ...n, selected: keys.has((n.data as StickyData).entity.key) };
      if (n.type === 'contextHeader') {
        const entity = (n.data as { entity?: { key: string } }).entity;
        return entity ? { ...n, selected: keys.has(entity.key) } : n;
      }
      if (n.type === 'lane') {
        const entity = (n.data as LaneData).entity;
        // Selection via data, not ReactFlow's selected flag, so
        // elevateNodesOnSelect never lifts a lane above the stickies.
        return entity ? { ...n, data: { ...n.data, selected: keys.has(entity.key) } } : n;
      }
      return n;
    });
  }, [nodes, selections, focusSets]);

  // Controlled React Flow still needs onNodesChange to apply its own changes
  // (measured dimensions, selection). Positions stay owned by the layout
  // engine: dragging is off so a sticky can only move programmatically.
  const [rfNodes, setRfNodes] = useState(displayNodes);
  useEffect(() => {
    setRfNodes(displayNodes);
  }, [displayNodes]);
  const onNodesChange = useCallback((changes: NodeChange<Node>[]) => {
    setRfNodes((nds) => applyNodeChanges(changes, nds));
  }, []);

  const previousFocus = useRef<FocusRequest | undefined>(undefined);
  const previousLayout = useRef(nodes);
  useEffect(() => {
    const previous = previousFocus.current;
    previousFocus.current = focus;
    const oldNodes = previousLayout.current;
    previousLayout.current = nodes;
    const layoutChanged =
      oldNodes.length !== nodes.length ||
      nodes.some((node, index) => {
        const old = oldNodes[index];
        return (
          !old ||
          old.id !== node.id ||
          old.type !== node.type ||
          old.parentId !== node.parentId ||
          old.hidden !== node.hidden ||
          old.position.x !== node.position.x ||
          old.position.y !== node.position.y ||
          (old.width ?? old.style?.width) !== (node.width ?? node.style?.width) ||
          (old.height ?? old.style?.height) !== (node.height ?? node.style?.height)
        );
      });
    const newFocus =
      focus &&
      (!previous ||
        focus.n !== previous.n ||
        focus.pending !== previous.pending ||
        (!focus.pending && !previous.pending && focus.key !== previous.key));
    if (focus?.pending || newFocus || (focus && !layoutChanged) || restored) return;
    if (nodes.length === 0) return;
    const t = setTimeout(() => fitView({ padding: 0.15, duration: 300 }), 50);
    return () => clearTimeout(t);
    // `nodes` is the layout output; depending on it (rather than `model`
    // alone) ensures we refit any time the rendered graph actually changes,
    // including dimension shifts that don't swap the model reference.
  }, [fitView, focus, nodes, restored]);

  return (
    <ReactFlow
      nodes={rfNodes}
      edges={displayEdges}
      nodeTypes={nodeTypes}
      fitView={!focus && !restored}
      minZoom={0.1}
      proOptions={{ hideAttribution: true }}
      onMoveEnd={onMoveEnd}
      onNodesChange={onNodesChange}
      onNodeClick={(e, node) => {
        const additive = e.metaKey || e.ctrlKey || e.shiftKey;
        if (node.type === 'sticky') {
          const d = node.data as StickyData;
          onSelect(d.entity.key, node.id, additive, d.instanceUid, d.moment?.key);
        } else if (node.type === 'sliceHeader' || node.type === 'storyboardHeader' || node.type === 'eventModelHeader')
          onSelect((node.data as StickyData).entity.key, undefined, additive);
        else if (node.type === 'lane') {
          // Persona/stream lanes are entities; other lanes act like the pane.
          const entity = (node.data as LaneData).entity;
          onSelect(entity?.key, undefined, additive && Boolean(entity));
        } else if (node.type === 'contextHeader') {
          const entity = (node.data as { entity?: { key: string } }).entity;
          if (entity) onSelect(entity.key, undefined, additive);
        }
      }}
      onEdgeClick={(e, edge) => {
        const additive = e.metaKey || e.ctrlKey || e.shiftKey;
        onSelectEdge({ id: edge.id, data: edge.data as BoardEdgeData }, additive);
      }}
      onPaneClick={() => onSelect(undefined)}
      nodesConnectable={false}
      nodesDraggable={false}
      elevateNodesOnSelect
    >
      <Background variant={BackgroundVariant.Dots} gap={18} size={1.2} color="#d4d4d8" />
      <Controls showInteractive={false} />
      <MiniMap
        pannable
        zoomable
        nodeColor={(n) => {
          // Lanes give the minimap the board's shape; stickies are tiny at
          // this scale, so their stroke (below) does the color work.
          if (n.type === 'lane') return (n.data as LaneData).tint;
          if (n.type === 'sticky') return MINIMAP_COLORS[(n.data as StickyData).entity.kind] ?? '#d4d4d8';
          return 'transparent';
        }}
        nodeStrokeWidth={24}
        nodeStrokeColor={(n) =>
          n.type === 'sticky' ? (MINIMAP_COLORS[(n.data as StickyData).entity.kind] ?? '#d4d4d8') : 'transparent'
        }
      />
    </ReactFlow>
  );
}

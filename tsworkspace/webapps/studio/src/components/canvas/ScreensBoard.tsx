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
import { Clock, Cog, Map as MapIcon, MousePointerClick, User } from 'lucide-react';
import { useMemo, useState } from 'react';
import type { SelectedEdge, Selection } from '@/components/canvas/Board';
import { BoardTopRight } from '@/components/canvas/BoardTopRight';
import { GlyphIcon } from '@/components/canvas/GlyphIcon';
import { ContextHeaderNode, StoryboardHeaderNode } from '@/components/canvas/StickyNode';
import type { BoardEdgeData } from '@/lib/layout';
import type { Model } from '@/lib/model';
import { buildScreensGroups, buildSitemapGroups } from '@/lib/multiboard';
import { FRAME_H, FRAME_W, type ScreenFrameData, SYS_H, type SystemStepData } from '@/lib/screens';
import { SITE_H, SITE_W, type SitemapNodeData } from '@/lib/sitemap';
import { cn } from '@/lib/utils';
import { typedNode } from './nodeTypes';
import { type FocusRequest, useFocusNode } from './useFocusNode';

const handleClass = '!h-1.5 !w-1.5 !border-0 !bg-zinc-400/70';

// One UI moment as designers think of it: any interaction surface (a GUI
// screen, a voice prompt, a CLI) framed with the data it presents and the
// actions it offers.
function ScreenFrameNode({ data }: NodeProps & { data: ScreenFrameData & { moment?: { id: { slug: string } } } }) {
  return (
    <div
      style={{ width: FRAME_W, height: FRAME_H }}
      className={cn(
        'flex cursor-pointer flex-col overflow-hidden rounded-lg border-2 border-zinc-400 bg-white shadow-[3px_4px_0_rgba(0,0,0,0.1)]',
        data.selected && 'ring-2 ring-ring',
      )}
    >
      <Handle id="l" type="target" position={Position.Left} className={handleClass} />
      <Handle id="r" type="source" position={Position.Right} className={handleClass} />
      <div className="flex items-center gap-1.5 border-b border-zinc-200 bg-zinc-100 px-2.5 py-1.5">
        <span className="flex gap-1">
          <span className="h-2 w-2 rounded-full bg-zinc-300" />
          <span className="h-2 w-2 rounded-full bg-zinc-300" />
          <span className="h-2 w-2 rounded-full bg-zinc-300" />
        </span>
        <span className="truncate text-[11px] font-semibold text-zinc-700">{data.ui.title}</span>
        {data.instance ? (
          <span
            title={`moment ${data.instance} of this UI: same surface, new state`}
            className="shrink-0 font-mono text-[9px] text-zinc-400"
          >
            {data.instance}
          </span>
        ) : null}
        {data.persona ? (
          <span className="ml-auto flex shrink-0 items-center gap-1 rounded bg-yellow-100 px-1 text-[9px] font-semibold text-yellow-800">
            <User className="h-2.5 w-2.5" />
            {data.persona.title}
          </span>
        ) : null}
      </div>
      <div className="flex-1 space-y-2 overflow-hidden px-2.5 py-2">
        {data.displays.map((rm) => (
          <div key={rm.key} className="rounded border border-emerald-200 bg-emerald-50/60 px-2 py-1.5">
            <div className="truncate text-[10px] font-semibold text-emerald-800">{rm.title}</div>
            {(rm.fields.length > 0 ? rm.fields.slice(0, 3) : [undefined, undefined]).map((f, i) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: skeleton rows
              <div key={i} className="mt-1 flex items-center gap-1.5">
                <span className="h-1.5 w-1/3 rounded-full bg-emerald-200/80" />
                <span className="text-[9px] font-mono text-emerald-700/70">{f ? String(f.name ?? '') : ''}</span>
              </div>
            ))}
            {rm.fields.length > 3 ? (
              <div className="mt-1 text-[8px] text-emerald-700/60">+{rm.fields.length - 3} more</div>
            ) : null}
          </div>
        ))}
        {data.displays.length === 0 ? (
          <div className="space-y-1.5 pt-1">
            <div className="h-1.5 w-3/4 rounded-full bg-zinc-200" />
            <div className="h-1.5 w-1/2 rounded-full bg-zinc-200" />
          </div>
        ) : null}
      </div>
      {data.actions.length > 0 ? (
        <div className="space-y-1.5 border-t border-zinc-200 px-2.5 py-2">
          {data.actions.map((cmd) => (
            <div key={cmd.key}>
              {cmd.fields.length > 0 ? (
                <div className="mb-1 truncate font-mono text-[8px] text-sky-700/70">
                  {cmd.fields
                    .slice(0, 4)
                    .map((f) => String(f.name ?? ''))
                    .join(' · ')}
                  {cmd.fields.length > 4 ? ` +${cmd.fields.length - 4}` : ''}
                </div>
              ) : null}
              <span className="inline-flex items-center gap-1 rounded-md bg-sky-500 px-2 py-1 text-[10px] font-semibold text-white">
                <MousePointerClick className="h-2.5 w-2.5" />
                {cmd.title}
              </span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

// What the machine does between two screen moments.
function SystemStepNode({
  data,
}: NodeProps & { data: SystemStepData & { moment?: { id: { namespace: string; slug: string } } } }) {
  return (
    <div
      style={{ width: FRAME_W, height: SYS_H }}
      className={cn(
        'cursor-pointer overflow-hidden rounded-lg border-2 border-dashed border-violet-300 bg-violet-50/80 px-2.5 py-2',
        data.selected && 'ring-2 ring-ring',
      )}
    >
      <Handle id="l" type="target" position={Position.Left} className={handleClass} />
      <Handle id="r" type="source" position={Position.Right} className={handleClass} />
      <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-violet-700">
        {data.lines.length > 1 ? <Clock className="h-3 w-3" /> : <Cog className="h-3 w-3" />}
        {data.lines.length > 1 ? `time passes: ${data.lines.length} moments` : 'meanwhile, the system…'}
      </div>
      <div className="mt-1 flex flex-col gap-0.5">
        {data.lines.slice(0, 3).map((line) => (
          <div key={line.text} className="flex items-center gap-1 text-[10px] leading-tight text-violet-900/80">
            {line.glyph ? <GlyphIcon glyph={line.glyph} className="h-2.5 w-2.5 shrink-0" /> : null}
            <span className="truncate">{line.text}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// One surface of the app, type-level: every moment of it collapsed into a
// single sitemap node. The journey mode shows the moments; this shows the map.
function SitemapScreenNode({ data }: NodeProps & { data: SitemapNodeData }) {
  return (
    <div
      style={{ width: SITE_W, height: SITE_H }}
      className={cn(
        'flex cursor-pointer flex-col overflow-hidden rounded-lg border-2 border-zinc-400 bg-white shadow-[3px_4px_0_rgba(0,0,0,0.1)]',
        data.selected && 'ring-2 ring-ring',
      )}
    >
      <Handle id="l" type="target" position={Position.Left} className={handleClass} />
      <Handle id="r" type="source" position={Position.Right} className={handleClass} />
      <div className="flex items-center gap-1.5 border-b border-zinc-200 bg-zinc-100 px-2.5 py-1.5">
        <span className="flex gap-1">
          <span className="h-2 w-2 rounded-full bg-zinc-300" />
          <span className="h-2 w-2 rounded-full bg-zinc-300" />
          <span className="h-2 w-2 rounded-full bg-zinc-300" />
        </span>
        <span className="truncate text-[11px] font-semibold text-zinc-700">{data.ui.title}</span>
        {data.instance ? (
          <span
            title={`occurrence ${data.instance} of this surface: same type, new moment (time only points forward)`}
            className="shrink-0 font-mono text-[9px] text-zinc-400"
          >
            {data.instance}
          </span>
        ) : null}
        {data.persona ? (
          <span className="ml-auto flex shrink-0 items-center gap-1 rounded bg-yellow-100 px-1 text-[9px] font-semibold text-yellow-800">
            <User className="h-2.5 w-2.5" />
            {data.persona.title}
          </span>
        ) : null}
      </div>
      <div className="flex flex-1 items-center gap-2 px-2.5 py-2 text-[10px] text-zinc-500">
        <span className="rounded bg-emerald-50 px-1.5 py-0.5 font-medium text-emerald-700">
          {data.displayCount} data
        </span>
        <span className="rounded bg-sky-50 px-1.5 py-0.5 font-medium text-sky-700">{data.actionCount} actions</span>
      </div>
    </div>
  );
}

const nodeTypes: NodeTypes = {
  screenFrame: typedNode(ScreenFrameNode),
  systemStep: typedNode(SystemStepNode),
  sitemapScreen: typedNode(SitemapScreenNode),
  seqStoryboardHeader: typedNode(StoryboardHeaderNode),
  contextHeader: typedNode(ContextHeaderNode),
};

export function ScreensBoard({
  model,
  selections,
  onSelect,
  onSelectEdge,
  focus,
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
}) {
  // Two readings of the same data: the journey (every UI moment in temporal
  // order) and the sitemap (surfaces as a navigable map, Overview-style). Mode
  // lives in the URL so studio state stays shareable.
  const [mode, setMode] = useState<'journey' | 'sitemap'>(() =>
    new URLSearchParams(window.location.search).get('uimode') === 'sitemap' ? 'sitemap' : 'journey',
  );
  const switchMode = (m: 'journey' | 'sitemap') => {
    setMode(m);
    const p = new URLSearchParams(window.location.search);
    if (m === 'sitemap') p.set('uimode', m);
    else p.delete('uimode');
    const qs = p.toString();
    window.history.replaceState(null, '', qs ? `?${qs}` : window.location.pathname);
  };
  const { nodes, edges } = useMemo(
    () => (mode === 'sitemap' ? buildSitemapGroups(model) : buildScreensGroups(model)),
    [model, mode],
  );
  useFocusNode(focus, nodes, mode === 'sitemap' ? 'sitemapScreen' : 'screenFrame');

  // A frame is one screen MOMENT: same type as its twins, not the same
  // thing: each reflects different state, otherwise the frame would not
  // exist. Canvas clicks pin the occurrence (occId = node id); drawer and
  // sidebar selections carry no occId and highlight the type everywhere.
  const displayNodes = useMemo(() => {
    const entitySel = selections.filter((s): s is Extract<Selection, { type: 'entity' }> => s.type === 'entity');
    const keys = new Set(entitySel.map((s) => s.key));
    const occIds = new Set(entitySel.filter((s) => s.occId).map((s) => s.occId));
    const keysWithoutOcc = new Set(entitySel.filter((s) => !s.occId).map((s) => s.key));
    return nodes.map((n) => {
      const entity = (n.data as { entity?: { key: string } }).entity;
      if (!entity) return n;
      // Frames, system steps, and sitemap occurrences are all moments:
      // same type as their twins, not the same thing. Canvas clicks pin
      // the occurrence; type-level selections highlight every twin.
      if (n.type === 'screenFrame' || n.type === 'systemStep' || n.type === 'sitemapScreen')
        return { ...n, data: { ...n.data, selected: occIds.has(n.id) || keysWithoutOcc.has(entity.key) } };
      return { ...n, selected: keys.has(entity.key) };
    });
  }, [nodes, selections]);

  const displayEdges = useMemo(() => {
    const edgeIds = new Set(selections.filter((s) => s.type === 'edge').map((s) => s.id));
    if (edgeIds.size === 0) return edges;
    return edges.map((e) =>
      edgeIds.has(e.id)
        ? { ...e, style: { ...e.style, strokeWidth: 3.5 }, zIndex: 20 }
        : { ...e, style: { ...e.style, opacity: 0.15 } },
    );
  }, [edges, selections]);

  return (
    <div className="relative h-full w-full">
      <ReactFlow
        key={mode}
        nodes={displayNodes}
        edges={displayEdges}
        nodeTypes={nodeTypes}
        fitView
        minZoom={0.05}
        proOptions={{ hideAttribution: true }}
        onNodeClick={(e, node) => {
          const entity = (node.data as { entity?: { key: string } }).entity;
          if (!entity) return;
          const additive = e.metaKey || e.ctrlKey || e.shiftKey;
          const isMoment = node.type === 'screenFrame' || node.type === 'systemStep' || node.type === 'sitemapScreen';
          const d = node.data as { instanceUid?: string; moment?: { key?: string } };
          onSelect(entity.key, isMoment ? node.id : undefined, additive, d.instanceUid, d.moment?.key);
        }}
        onEdgeClick={(e, edge) => {
          const additive = e.metaKey || e.ctrlKey || e.shiftKey;
          onSelectEdge({ id: edge.id, data: edge.data as BoardEdgeData }, additive);
        }}
        onPaneClick={() => onSelect(undefined)}
        nodesConnectable={false}
        nodesDraggable={false}
        elevateNodesOnSelect={false}
      >
        <Background variant={BackgroundVariant.Dots} gap={18} size={1.2} color="#d4d4d8" />
        <Controls showInteractive={false} />
      </ReactFlow>
      <BoardTopRight>
        <div className="flex overflow-hidden rounded-md border border-border bg-card text-xs shadow-sm">
          {(['journey', 'sitemap'] as const).map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => switchMode(m)}
              className={`flex items-center gap-1 px-3 py-1.5 font-medium capitalize ${mode === m ? 'bg-foreground text-background' : 'hover:bg-accent'}`}
            >
              {m === 'sitemap' ? <MapIcon className="h-3 w-3" /> : null}
              {m}
            </button>
          ))}
        </div>
      </BoardTopRight>
      <div className="absolute bottom-3 right-3 rounded-md border border-border bg-card/95 px-2.5 py-2 text-[10px]">
        <div className="flex items-center gap-1.5">
          <span className="inline-block h-0.5 w-4 bg-zinc-400" />
          {mode === 'sitemap'
            ? 'the app surface: every UI, edges labeled by the command that navigates'
            : "the user's journey: each UI moment, what it presents, what it offers"}
        </div>
      </div>
    </div>
  );
}

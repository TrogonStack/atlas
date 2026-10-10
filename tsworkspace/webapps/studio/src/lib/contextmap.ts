import { type Edge, MarkerType, type Node } from '@xyflow/react';
import { contextSeams, type Entity, type Model, relationshipsOf } from './model';

export const CONTEXT_MAP_W = 220;
export const CONTEXT_MAP_H = 76;

export interface ContextMapNodeData extends Record<string, unknown> {
  entity: Entity;
  inbound: number;
  outbound: number;
  selected?: boolean;
}

export interface ContextMapEdgeData extends Record<string, unknown> {
  upstream: Entity;
  downstream: Entity;
  views: string[];
  events: string[];
  intent?: string;
}

interface SeamGroup {
  upstream: string;
  downstream: string;
  views: Set<string>;
  events: Set<string>;
}

export function buildContextMap(model: Model): { nodes: Node[]; edges: Edge[] } {
  const contexts = model.entities.filter((e) => e.kind === 'boundedContext');
  const byNamespace = new Map(contexts.map((ctx) => [ctx.id.namespace, ctx]));
  const groups = new Map<string, SeamGroup>();

  for (const seam of contextSeams(model)) {
    const upstream = seam.upstreamEvent.namespace;
    const downstream = seam.view.id.namespace;
    if (!byNamespace.has(upstream) || !byNamespace.has(downstream)) continue;
    const key = `${upstream}->${downstream}`;
    const group =
      groups.get(key) ??
      ({
        upstream,
        downstream,
        views: new Set<string>(),
        events: new Set<string>(),
      } satisfies SeamGroup);
    group.views.add(seam.view.title);
    group.events.add(seam.upstreamEvent.slug);
    groups.set(key, group);
  }

  const upstreamsOf = new Map<string, Set<string>>();
  const inbound = new Map<string, number>();
  const outbound = new Map<string, number>();
  for (const group of groups.values()) {
    const ups = upstreamsOf.get(group.downstream) ?? new Set<string>();
    ups.add(group.upstream);
    upstreamsOf.set(group.downstream, ups);
    inbound.set(group.downstream, (inbound.get(group.downstream) ?? 0) + group.views.size);
    outbound.set(group.upstream, (outbound.get(group.upstream) ?? 0) + group.views.size);
  }

  const depthCache = new Map<string, number>();
  const depthOf = (ns: string, trail = new Set<string>()): number => {
    const cached = depthCache.get(ns);
    if (cached !== undefined) return cached;
    if (trail.has(ns)) return 0;
    const nextTrail = new Set(trail);
    nextTrail.add(ns);
    const upstreams = [...(upstreamsOf.get(ns) ?? [])];
    const depth = upstreams.length === 0 ? 0 : Math.max(...upstreams.map((up) => depthOf(up, nextTrail))) + 1;
    depthCache.set(ns, depth);
    return depth;
  };

  const byDepth = new Map<number, Entity[]>();
  for (const ctx of contexts) {
    const depth = depthOf(ctx.id.namespace);
    const bucket = byDepth.get(depth) ?? [];
    bucket.push(ctx);
    byDepth.set(depth, bucket);
  }

  const positionByNamespace = new Map<string, { x: number; y: number }>();
  const nodes: Node[] = [];
  for (const [depth, bucket] of [...byDepth.entries()].sort((a, b) => a[0] - b[0])) {
    bucket.sort((a, b) => a.title.localeCompare(b.title));
    bucket.forEach((ctx, row) => {
      const position = {
        x: depth * (CONTEXT_MAP_W + 180),
        y: row * (CONTEXT_MAP_H + 56),
      };
      positionByNamespace.set(ctx.id.namespace, position);
      nodes.push({
        id: `ctx:${ctx.id.namespace}`,
        type: 'contextMapNode',
        position,
        style: { width: CONTEXT_MAP_W, height: CONTEXT_MAP_H },
        data: {
          entity: ctx,
          inbound: inbound.get(ctx.id.namespace) ?? 0,
          outbound: outbound.get(ctx.id.namespace) ?? 0,
        } satisfies ContextMapNodeData,
        draggable: false,
      });
    });
  }

  const edges: Edge[] = [];
  for (const group of groups.values()) {
    const upstream = byNamespace.get(group.upstream);
    const downstream = byNamespace.get(group.downstream);
    if (!upstream || !downstream) continue;
    const rel = relationshipsOf(downstream).find(
      (r) => r.upstream.namespace === upstream.id.namespace && r.upstream.slug === upstream.id.slug,
    );
    edges.push({
      id: `seam:${group.upstream}->${group.downstream}`,
      source: `ctx:${group.upstream}`,
      target: `ctx:${group.downstream}`,
      type: 'smoothstep',
      animated: true,
      markerEnd: { type: MarkerType.ArrowClosed, color: '#d946ef' },
      style: { stroke: '#d946ef', strokeWidth: 1.8 },
      label: `${group.views.size} view${group.views.size === 1 ? '' : 's'}`,
      data: {
        upstream,
        downstream,
        views: [...group.views].sort(),
        events: [...group.events].sort(),
        intent: rel?.intent,
      } satisfies ContextMapEdgeData,
    });
  }

  return { nodes, edges };
}

// Multi-context composition: a bounded context is its own thing. With
// several namespaces on screen, each event model lays out as a fully
// self-contained group (its own personas, lanes, and timeline), stacked
// under a context header band. The ONLY cross-group connections are the
// context seams: upstream events feeding downstream subscription views.
// Never one gigantic merged board.
import type { Edge, Node } from '@xyflow/react';
import { layoutBoard, NODE_H, NODE_W } from './layout';
import { contextSeams, type Entity, type Model, partitionByNamespace, slugKey } from './model';
import { buildScreens } from './screens';
import { buildSequence } from './sequence';
import { buildSitemap } from './sitemap';

// Multi-board group-header band height. Deliberately smaller than
// `CTX_H` in `domainchart.ts` (56): that constant sizes a top-level
// BoundedContext card in the Domain Chart, while this one sizes the
// thin label strip above each board group on the multi-board view.
// Different visual elements; the name collision is incidental.
const MULTIBOARD_GROUP_HEADER_H = 44;
const GROUP_GAP = 140;

function nodeBottom(n: Node): number {
  const style = (n.style ?? {}) as { height?: number | string };
  return n.position.y + (Number(n.height ?? style.height ?? NODE_H) || NODE_H);
}

function composeGroups(
  model: Model,
  build: (part: Model) => { nodes: Node[]; edges: Edge[] },
  opts?: { temporal?: boolean },
): { nodes: Node[]; edges: Edge[] } {
  const parts = partitionByNamespace(model);
  if (parts.length === 1) return build(model);

  const seams = contextSeams(model);

  // Time flows across contexts too: order groups upstream-first and shift
  // each downstream group right so its subscription view lands just AFTER
  // the upstream event it consumes: the canvas reads as one timeline, not
  // as left-aligned silos.
  let ordered = parts;
  if (opts?.temporal && seams.length > 0) {
    const upstreamsOf = new Map<string, Set<string>>();
    for (const seam of seams) {
      const set = upstreamsOf.get(seam.view.id.namespace) ?? new Set<string>();
      set.add(seam.upstreamEvent.namespace);
      upstreamsOf.set(seam.view.id.namespace, set);
    }
    const placedNs = new Set<string>();
    const remaining = [...parts];
    ordered = [];
    while (remaining.length > 0) {
      const i = remaining.findIndex((p) =>
        [...(upstreamsOf.get(p.namespaces[0] ?? '') ?? [])].every(
          (up) => placedNs.has(up) || !parts.some((q) => q.namespaces[0] === up),
        ),
      );
      const next = remaining.splice(i >= 0 ? i : 0, 1)[0];
      placedNs.add(next.namespaces[0] ?? '');
      ordered.push(next);
    }
  }

  const nodes: Node[] = [];
  const edges: Edge[] = [];
  const findPlaced = (key: string) =>
    nodes.find((n) => {
      const e = (n.data as { entity?: Entity }).entity;
      return e && slugKey(e.kind, e.id) === key && (n.type === 'sticky' || n.type === 'seqSticky');
    });
  let yOff = 0;
  for (const part of ordered) {
    const ns = part.namespaces[0] ?? '?';
    // A halo-only partition holds nothing but foreign source events pulled
    // in by the /api/event-model halo so seams have a source. Don't render
    // it at all: no context band, no borrowed event cards, and (since the
    // source node never lands on the board) no seam arrow. The fuchsia
    // subscription marker on the read model card carries the story.
    if (part.entities.every((e) => e.kind === 'event' || e.kind === 'boundedContext')) continue;
    const l = build(part);
    // A namespace with no behavioral content, typically a knowledge-graph
    // partition (Domain / Subdomain / sibling BC) merged in for the Domain
    // tab, would otherwise render as an empty, unclickable context band:
    // the lane skeleton is built unconditionally, so l.nodes is never zero.
    // Require at least one content node (any view-specific kind) to keep
    // the band. UI / sitemap / sequence views produce their own node
    // types (screenFrame, systemStep, sitemapScreen, seqSticky, etc.)
    // so this list must cover every view's content kinds, otherwise the
    // partition that holds the real content gets dropped as "empty".
    const CONTENT_NODE_TYPES = new Set([
      // Board view
      'sticky',
      'sliceHeader',
      'storyboardHeader',
      'eventModelHeader',
      // Sequence view
      'seqSticky',
      'seqSliceHeader',
      'seqStoryboardHeader',
      'seqLoopStub',
      // UI view
      'screenFrame',
      'systemStep',
      // Sitemap view
      'sitemapScreen',
    ]);
    const hasContent = l.nodes.some((n) => CONTENT_NODE_TYPES.has(n.type ?? ''));
    if (!hasContent) continue;
    let xShift = 0;
    if (opts?.temporal) {
      for (const seam of seams.filter((sm) => sm.view.id.namespace === ns)) {
        const upNode = findPlaced(slugKey('event', seam.upstreamEvent));
        const viewNode = l.nodes.find((n) => {
          const e = (n.data as { entity?: Entity }).entity;
          return e && slugKey(e.kind, e.id) === slugKey('readModel', seam.view.id) && n.type === 'seqSticky';
        });
        if (upNode && viewNode) {
          xShift = Math.max(xShift, upNode.position.x + NODE_W + 56 - viewNode.position.x);
        }
      }
    }
    // `Math.min(...arr)` and `Math.max(...arr)` throw `RangeError` past
    // ~100k elements (engine-dependent call-stack limit). Use `.reduce`
    // so boards built from very large stores don't crash the renderer.
    const minY = l.nodes.reduce((acc, n) => Math.min(acc, n.position.y), Infinity);
    const minX = l.nodes.reduce((acc, n) => Math.min(acc, n.position.x), Infinity);
    const maxY = l.nodes.reduce((acc, n) => Math.max(acc, nodeBottom(n)), -Infinity);
    const maxRight = l.nodes.reduce((acc, n) => {
      const style = (n.style ?? {}) as { width?: number | string };
      const right = n.position.x + (Number(n.width ?? style.width ?? NODE_W) || NODE_W);
      return Math.max(acc, right);
    }, -Infinity);
    // The band is the context's ROOT DOCUMENT: the BoundedContext entity
    // (Decision #27). Like a storyboard header, it spans exactly what
    // belongs to it: the group's bounding box.
    const bc = part.entities.find((e) => e.kind === 'boundedContext');
    nodes.push({
      id: `ctx:${ns}`,
      type: 'contextHeader',
      position: { x: minX + xShift, y: yOff },
      style: { width: Math.max(maxRight - minX, 480), height: MULTIBOARD_GROUP_HEADER_H },
      data: { ns, title: bc?.title, entity: bc },
      draggable: false,
      zIndex: 1,
    });
    const shift = yOff + MULTIBOARD_GROUP_HEADER_H + 28 - minY;
    for (const n of l.nodes) {
      nodes.push({ ...n, id: `${ns}::${n.id}`, position: { x: n.position.x + xShift, y: n.position.y + shift } });
    }
    for (const e of l.edges) {
      edges.push({ ...e, id: `${ns}::${e.id}`, source: `${ns}::${e.source}`, target: `${ns}::${e.target}` });
    }
    yOff += MULTIBOARD_GROUP_HEADER_H + 28 + (maxY - minY) + GROUP_GAP;
  }

  // Cross-context seams DO draw, as exactly one dashed boundary-colored
  // tie per subscription: upstream event down into the downstream view's
  // first occurrence. An earlier iteration relied on the fuchsia icon on
  // the read model card alone ("inter-context flow is structural, not
  // visual clutter"), which left the upstream event rendering as an
  // orphan in its context band with nothing explaining why it is on the
  // board at all. One seam arrow per subscription is the story the board
  // exists to tell; the dash + boundary color keep it visually distinct
  // from intra-context projection ties.
  const seamKey = (up: string, view: string) => `${up}->${view}`;
  const drawnSeams = new Set<string>();
  for (const seam of seams) {
    const upKey = slugKey('event', seam.upstreamEvent);
    const viewKey = slugKey('readModel', seam.view.id);
    if (drawnSeams.has(seamKey(upKey, viewKey))) continue;
    const upNode = findPlaced(upKey);
    // A read model occurs once per touching slice; the seam feeds the
    // subscription's ENTRY moment, so tie to the leftmost occurrence.
    const viewNode = nodes.reduce<Node | undefined>((best, n) => {
      const e = (n.data as { entity?: Entity }).entity;
      if (!e || slugKey(e.kind, e.id) !== viewKey) return best;
      if (n.type !== 'sticky' && n.type !== 'seqSticky') return best;
      return !best || n.position.x < best.position.x ? n : best;
    }, undefined);
    if (!upNode || !viewNode) continue;
    drawnSeams.add(seamKey(upKey, viewKey));
    edges.push({
      id: `seam:${upKey}->${viewKey}`,
      source: upNode.id,
      target: viewNode.id,
      sourceHandle: 'ts',
      targetHandle: 'bt',
      animated: true,
      style: { stroke: '#d946ef', strokeWidth: 1.8, strokeDasharray: '7 5' },
      data: {
        relation: 'feeds context',
        doc: '',
        metadata: [],
        source: (upNode.data as { entity?: Entity }).entity,
        target: seam.view,
      },
    });
  }
  return { nodes, edges };
}

export function layoutBoards(model: Model): { nodes: Node[]; edges: Edge[] } {
  return composeGroups(model, layoutBoard);
}

export function buildSequences(model: Model): { nodes: Node[]; edges: Edge[] } {
  return composeGroups(model, buildSequence, { temporal: true });
}

export function buildScreensGroups(model: Model): { nodes: Node[]; edges: Edge[] } {
  return composeGroups(model, buildScreens);
}

export function buildSitemapGroups(model: Model): { nodes: Node[]; edges: Edge[] } {
  return composeGroups(model, buildSitemap);
}

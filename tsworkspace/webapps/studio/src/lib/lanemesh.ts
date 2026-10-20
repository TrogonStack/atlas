// The lane mesh: the Board in 3D. Every card keeps the Board's lane row and
// slice column, read from the Board's own layout, so the two views always
// agree about where a moment sits. Depth adds what a flat board cannot show:
// links arc out of the board toward the viewer, so a link does not run across
// the cards between its ends, and cite links run behind the lanes to the
// code the model cites, on a wall under the board.

import type { Edge, Node } from '@xyflow/react';
import { codeRefsOf, FILE_SIZE, layoutCodeWall, type WallEntry } from './codewall';
import {
  type BoardEdgeData,
  type GapHeaderData,
  type LaneData,
  NODE_H,
  NODE_W,
  type SliceHeaderData,
  type StickyData,
} from './layout';
import { type Entity, isSeamEdge, type Model, orphanOf } from './model';
import { layoutBoards, namespaceOf } from './multiboard';
import {
  add,
  type Body,
  cubicLink,
  distance,
  linkPorts,
  normalize,
  type PortLink,
  scale,
  type Vec3,
  vec,
} from './ports';
import {
  type Region,
  regionOf,
  type SceneDecor,
  type SceneFile,
  type SceneLink,
  type SceneNode,
  type ScenePlate,
  type SceneSpec,
  type SceneStep,
  type SceneTag,
  unionRegions,
} from './scene3d';

/** World units per Board pixel: a Board card becomes a 54 by 30.5 card. */
export const PX = 0.25;
export const CARD_DEPTH = 3;
// Plates and lane bands sit just behind the cards, so a card never fights a band for the same depth.
const PLATE_Z = -(CARD_DEPTH / 2) - 0.9;
const BAND_Z = PLATE_Z - 0.8;
/** How far under the lowest lane the code wall hangs. */
export const WALL_DROP = 40;
/**
 * How far behind the cards the code wall stands. Cite links run at this
 * depth, behind the lane bands, so they do not cross the cards of the lanes
 * below.
 */
export const WALL_BACK = 24;
const FILE_DEPTH = 6;

interface PxBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

function pxBox(n: Node): PxBox {
  const style = (n.style ?? {}) as { width?: number | string; height?: number | string };
  return {
    x: n.position.x,
    y: n.position.y,
    w: Number(n.width ?? style.width ?? NODE_W) || NODE_W,
    h: Number(n.height ?? style.height ?? NODE_H) || NODE_H,
  };
}

// The Board grows down and the scene grows up, so y flips.
const centerOf = (b: PxBox, z: number): Vec3 => vec((b.x + b.w / 2) * PX, -(b.y + b.h / 2) * PX, z);
const cornersOf = (b: PxBox, z0: number, z1: number): Vec3[] => [
  vec(b.x * PX, -(b.y + b.h) * PX, z0),
  vec((b.x + b.w) * PX, -b.y * PX, z1),
];

const LANE_KICKER: Partial<Record<string, string>> = { persona: 'Persona', stream: 'Event stream' };

export function buildLaneMesh(model: Model): SceneSpec {
  const { nodes: boardNodes, edges: boardEdges } = layoutBoards(model);
  const nodes: SceneNode[] = [];
  const plates: ScenePlate[] = [];
  const decor: SceneDecor[] = [];
  const corners: Vec3[] = [];
  const lanesByGroup = new Map<string, PxBox[]>();
  const cardBox = new Map<string, PxBox>();
  // Each group's lanes end where its own first column starts, wherever the group sits on the Board.
  const firstColumn = new Map<string, number>();
  for (const n of boardNodes) {
    if (n.type !== 'sticky') continue;
    const g = namespaceOf(n.id);
    firstColumn.set(g, Math.min(firstColumn.get(g) ?? Infinity, n.position.x));
  }

  for (const n of boardNodes) {
    const b = pxBox(n);
    if (n.type === 'sticky') {
      const d = n.data as StickyData;
      cardBox.set(n.id, b);
      nodes.push({
        id: n.id,
        entity: d.entity,
        body: {
          id: n.id,
          center: centerOf(b, 0),
          shape: { type: 'box', width: b.w * PX, height: b.h * PX, depth: CARD_DEPTH },
        },
        printed: true,
        occId: n.id,
        instanceUid: d.instanceUid,
        momentKey: d.moment?.key,
        instance: d.instance,
        synthetic: d.synthetic,
        flag: d.multipleIssuers
          ? 'error'
          : d.noEmittedEvents
            ? 'pending'
            : orphanOf(d.entity) !== undefined
              ? 'orphan'
              : undefined,
      });
      corners.push(...cornersOf(b, -CARD_DEPTH / 2, CARD_DEPTH / 2));
      continue;
    }
    if (n.type === 'lane') {
      const d = n.data as LaneData;
      const group = namespaceOf(n.id);
      lanesByGroup.set(group, [...(lanesByGroup.get(group) ?? []), b]);
      decor.push({
        type: 'plane',
        id: `band:${n.id}`,
        center: centerOf(b, BAND_Z),
        width: b.w * PX,
        height: b.h * PX,
        color: d.tint,
        opacity: 0.9,
        facing: 'z',
      });
      // The label takes the strip left of the first column, as on the Board.
      const labelW = Math.min(Math.max((firstColumn.get(group) ?? 0) - b.x, 120), 224) - 12;
      const labelH = Math.min(b.h - 12, 64);
      plates.push({
        id: `lane:${n.id}`,
        center: centerOf({ x: b.x + 6, y: b.y + 6, w: labelW, h: labelH }, PLATE_Z),
        width: labelW * PX,
        height: labelH * PX,
        kicker: d.glyph ? LANE_KICKER[d.glyph] : undefined,
        title: d.label,
        sub: d.entity?.streamId,
        tint: d.tint,
        entity: d.entity,
      });
      corners.push(...cornersOf(b, BAND_Z, BAND_Z));
      continue;
    }
    const plate = headerPlate(n, b);
    if (plate) {
      plates.push(plate);
      corners.push(...cornersOf(b, PLATE_Z, PLATE_Z));
    }
  }

  const byId = new Map(nodes.map((n) => [n.id, n]));
  const edges = boardEdges.filter((e) => byId.has(e.source) && byId.has(e.target) && e.source !== e.target);

  // The first occurrence of an entity carries its cite links: one link per
  // file is enough to show where the code is, and the later moments of the
  // same read model or screen would only repeat it.
  const firstCard = new Map<string, SceneNode>();
  for (const n of nodes) {
    const seen = firstCard.get(n.entity.key);
    if (!seen || n.body.center.x < seen.body.center.x) firstCard.set(n.entity.key, n);
  }
  const wallZ = -WALL_BACK;
  const wallEntries: WallEntry[] = [];
  for (const [key, card] of firstCard) {
    const refs = codeRefsOf(card.entity);
    if (refs.length > 0) wallEntries.push({ key, x: card.body.center.x, refs });
  }
  const board = regionOf(corners);
  const wall = layoutCodeWall(wallEntries, { top: board.min.y - WALL_DROP, z: wallZ });
  const files: SceneFile[] = (wall?.files ?? []).map((f) => ({
    id: f.id,
    body: { id: f.id, center: f.center, shape: { type: 'box', width: f.size, height: f.size, depth: FILE_DEPTH } },
    repo: f.repo,
    path: f.path,
    symbols: f.symbols,
    citedBy: f.citedBy,
  }));
  const fileById = new Map(files.map((f) => [f.id, f]));
  const cites = (wall?.files ?? []).flatMap((f) =>
    f.citedBy.flatMap((key) => {
      const card = firstCard.get(key);
      return card ? [{ card, file: fileById.get(f.id) as SceneFile }] : [];
    }),
  );

  const portLinks: PortLink[] = [
    ...edges.map((e) => ({ from: bodyOf(byId, e.source), to: bodyOf(byId, e.target) })),
    // Aiming far below the card keeps the y axis winning in boxPort, so the link leaves from the bottom rim at any file distance.
    ...cites.map(({ card, file }) => ({
      from: card.body,
      to: file.body,
      fromToward: vec(card.body.center.x, card.body.center.y - 1_000_000, card.body.center.z),
    })),
  ];
  const ports = linkPorts(portLinks);
  const links: SceneLink[] = [];
  edges.forEach((e, i) => {
    const { from, to } = ports[i];
    const d = distance(from.point, to.point);
    links.push({
      id: e.id,
      from: e.source,
      to: e.target,
      points: cubicLink(from, to, Math.min(60, d * 0.4), vec(0, 0, 10 + d * 0.16)),
      color: strokeOf(e),
      radius: isSeamEdge(e.id) ? 0.8 : 0.55,
      kind: 'edge',
      edge: e.data as BoardEdgeData,
    });
  });
  cites.forEach(({ card, file }, i) => {
    const { from, to } = ports[edges.length + i];
    const drop = Math.min(60, distance(from.point, to.point) * 0.4);
    links.push({
      id: `cite:${card.id}->${file.id}`,
      from: card.id,
      to: file.id,
      points: [
        from.point,
        add(add(from.point, scale(from.normal, 12)), vec(0, 0, wallZ)),
        add(to.point, scale(to.normal, drop)),
        to.point,
      ],
      color: '#71717a',
      radius: 0.3,
      kind: 'cite',
    });
  });

  const tags: SceneTag[] = [];
  if (wall) {
    decor.push({
      type: 'plane',
      id: 'wall',
      center: vec((wall.region.min.x + wall.region.max.x) / 2, (wall.region.min.y + wall.region.max.y) / 2, wallZ - 6),
      width: wall.region.max.x - wall.region.min.x,
      height: wall.region.max.y - wall.region.min.y,
      color: '#f4f4f5',
      opacity: 0.94,
      facing: 'z',
    });
    for (const b of wall.bundles) {
      decor.push({
        type: 'plane',
        id: `bundle:${b.id}`,
        center: vec(b.center.x, b.center.y, wallZ - 4),
        width: b.width,
        height: b.height,
        color: '#e4e4e7',
        opacity: 0.96,
        facing: 'z',
      });
      tags.push({
        id: `bundle:${b.id}`,
        position: vec(b.center.x, b.center.y + b.height / 2 - 5, wallZ),
        text: b.label,
        tone: 'zone',
        priority: 6,
        related: [...new Set(b.files.flatMap((id) => fileById.get(id)?.citedBy ?? []))],
      });
    }
    for (const f of wall.files) {
      tags.push({
        id: `name:${f.id}`,
        position: vec(f.center.x, f.center.y - FILE_SIZE / 2 - 2.5, wallZ),
        text: f.name,
        tone: 'file',
        priority: 1,
        related: f.citedBy,
      });
    }
    tags.push({
      id: 'wall-title',
      position: vec(wall.region.min.x + 40, wall.region.max.y + 6, wallZ),
      text: `Code the model cites: ${files.length} ${files.length === 1 ? 'file' : 'files'}`,
      tone: 'ghost',
      priority: 7,
    });
  }

  const front = wall
    ? unionRegions([
        regionOf(corners, 8),
        wall.region,
        regionOf([vec(wall.region.min.x, wall.region.max.y + 14, wallZ)]),
      ])
    : regionOf(corners, 8);
  const steps = sliceSteps(boardNodes, cardBox, lanesByGroup);
  return {
    nodes,
    files,
    links,
    plates,
    tags,
    decor,
    front,
    frontDirection: vec(0, 0, 1),
    overview: {
      region: front,
      direction: normalize(vec(-0.42, 0.3, 1)),
    },
    steps,
    stepDirection: normalize(vec(-0.55, 0.24, 1)),
    stepNoun: 'Slice',
    legend: [
      'Columns are slices in storyboard order. Rows are the Board lanes.',
      'Links arc in front of the cards.',
      ...(wall ? ['Under the board: the code the model cites. Cite links run behind the lanes.'] : []),
    ],
  };
}

function bodyOf(byId: Map<string, SceneNode>, id: string): Body {
  return (byId.get(id) as SceneNode).body;
}

function strokeOf(e: Edge): string {
  const stroke = (e.style as { stroke?: unknown } | undefined)?.stroke;
  return typeof stroke === 'string' ? stroke : '#a1a1aa';
}

function headerPlate(n: Node, b: PxBox): ScenePlate | undefined {
  const center = centerOf(b, PLATE_Z);
  const size = { width: b.w * PX, height: b.h * PX };
  switch (n.type) {
    case 'sliceHeader': {
      const d = n.data as SliceHeaderData;
      return { id: n.id, center, ...size, kind: d.entity.kind, title: d.entity.title, sub: d.status, entity: d.entity };
    }
    case 'storyboardHeader':
    case 'eventModelHeader': {
      const d = n.data as StickyData;
      return { id: n.id, center, ...size, kind: d.entity.kind, title: d.entity.title, entity: d.entity };
    }
    case 'gapHeader': {
      const d = n.data as GapHeaderData;
      return {
        id: n.id,
        center,
        ...size,
        kicker: 'Implicit projection: no slice yet',
        title: `${d.readModels.join(', ')} updated by ${d.events.join(', ')}`,
        tint: '#f4f4f5',
        border: '#a1a1aa',
        dashed: true,
      };
    }
    case 'contextHeader': {
      const d = n.data as { ns: string; title?: string; entity?: Entity };
      return {
        id: n.id,
        center,
        ...size,
        kicker: 'Bounded context',
        title: d.title ? `${d.ns} · ${d.title}` : d.ns,
        tint: '#fdf4ff',
        border: '#f0abfc',
        entity: d.entity,
      };
    }
    default:
      return undefined;
  }
}

/**
 * One step per slice column, in the Board's order. A column spans the cards
 * whose moment is that slice; a slice with no cards keeps its header's
 * place, so an empty column is still a stop on the way.
 */
function sliceSteps(boardNodes: Node[], cardBox: Map<string, PxBox>, lanesByGroup: Map<string, PxBox[]>): SceneStep[] {
  const cardsBySlice = new Map<string, PxBox[]>();
  for (const n of boardNodes) {
    if (n.type !== 'sticky') continue;
    const moment = (n.data as StickyData).moment;
    const b = cardBox.get(n.id);
    if (!moment || !b || (n.data as StickyData).synthetic) continue;
    const key = `${namespaceOf(n.id)}\0${moment.key}`;
    cardsBySlice.set(key, [...(cardsBySlice.get(key) ?? []), b]);
  }
  const steps: SceneStep[] = [];
  for (const n of boardNodes) {
    if (n.type !== 'sliceHeader') continue;
    const slice = (n.data as SliceHeaderData).entity;
    const group = namespaceOf(n.id);
    const header = pxBox(n);
    const cards = cardsBySlice.get(`${group}\0${slice.key}`) ?? [];
    const lanes = lanesByGroup.get(group) ?? [];
    const x0 = cards.length > 0 ? Math.min(...cards.map((c) => c.x)) : header.x + header.w / 2 - NODE_W / 2;
    const x1 = cards.length > 0 ? Math.max(...cards.map((c) => c.x + c.w)) : header.x + header.w / 2 + NODE_W / 2;
    const top = header.y;
    const bottom = lanes.reduce((m, l) => Math.max(m, l.y + l.h), header.y + header.h);
    const column: PxBox = { x: x0 - 12, y: top, w: x1 - x0 + 24, h: bottom - top };
    const region: Region = regionOf(cornersOf(column, BAND_Z, CARD_DEPTH * 4));
    steps.push({
      label: slice.title,
      region,
      highlight: { center: centerOf(column, BAND_Z + 0.4), width: column.w * PX, height: column.h * PX, facing: 'z' },
    });
  }
  return steps;
}

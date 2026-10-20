// The stream towers: every event stream as a tower. A stream's events stack
// as floors from the bottom up, in the Board's storyboard and slice order, so
// a tower reads as the history of one stream. Floors line up across towers:
// one height is one Board column, so moments in different streams compare at
// a glance. Commands that append to a stream sit on its left, read models
// that project from it on its right, and the automations that act on it in
// front. Everything is read from the Board's layout, so a selection carries
// across views by occurrence id.

import type { Node } from '@xyflow/react';
import { flagOf, type PxBox, pxBox, strokeOf } from './lanemesh';
import type { LaneData, SliceHeaderData, StickyData } from './layout';
import { isSeamEdge, type Model } from './model';
import { layoutBoards, namespaceOf } from './multiboard';
import { add, type Body, distance, linkPorts, normalize, scale, type Vec3, vec } from './ports';
import {
  type Region,
  regionOf,
  type SceneDecor,
  type SceneLink,
  type SceneNode,
  type ScenePlate,
  type SceneSpec,
  type SceneStep,
  type SceneTag,
} from './scene3d';

/** Distance between two tower axes, wide enough for a read model and the next tower's command side by side. */
export const TOWER_GAP = 170;
/** Event discs. Thin, so links leave from the rim and the floor above stays visible. */
const DISC_R = 11;
const DISC_H = 2.4;
/**
 * How far commands, read models and automations sit from their tower's
 * axis. Far enough that the link from a command to its event shows between
 * the card and the disc.
 */
export const SIDE_R = 56;
const CARD_W = 48;
const CARD_H = 16;
const CARD_D = 2;
/** Height of one floor, and of each extra row when a floor holds several items of one kind. */
export const FLOOR = 24;
const SUB_FLOOR = 18;

type Role = 'event' | 'command' | 'readModel' | 'processor';

// Degrees around the tower, counterclockwise from +x seen from above; 90 faces the Front preset.
const ANGLE: Record<Exclude<Role, 'event'>, number> = { command: 150, readModel: 30, processor: 90 };

interface Placed {
  node: Node;
  data: StickyData;
  box: PxBox;
  role: Role;
  tower: number;
}

export function buildTowers(model: Model): SceneSpec {
  const { nodes: boardNodes, edges: boardEdges } = layoutBoards(model);

  // One tower per stream lane, in the Board's lane order. The Board's catch-all
  // lane for events without a swimlane becomes a tower too, so no event is lost.
  const lanes = boardNodes.filter((n) => n.type === 'lane' && (n.data as LaneData).glyph === 'stream');
  const towerOf = (n: Node): number => {
    const b = pxBox(n);
    const cy = b.y + b.h / 2;
    return lanes.findIndex((l) => {
      const lb = pxBox(l);
      return namespaceOf(l.id) === namespaceOf(n.id) && cy >= lb.y && cy < lb.y + lb.h;
    });
  };

  const stickies = new Map(boardNodes.filter((n) => n.type === 'sticky').map((n) => [n.id, n]));
  const outgoing = new Map<string, string[]>();
  const incoming = new Map<string, string[]>();
  for (const e of boardEdges) {
    outgoing.set(e.source, [...(outgoing.get(e.source) ?? []), e.target]);
    incoming.set(e.target, [...(incoming.get(e.target) ?? []), e.source]);
  }

  const placed = new Map<string, Placed>();
  const place = (n: Node, role: Role, tower: number) =>
    placed.set(n.id, { node: n, data: n.data as StickyData, box: pxBox(n), role, tower });
  for (const n of stickies.values()) {
    if ((n.data as StickyData).entity.kind !== 'event') continue;
    const tower = towerOf(n);
    if (tower >= 0) place(n, 'event', tower);
  }
  // A command joins the stream it appends to; a read model the stream it
  // projects from. When there are several, the one with the most links wins,
  // and the others are reached by links that arc across.
  const towerFor = (ids: string[] | undefined, role: Role): number | undefined => {
    const counts = new Map<number, number>();
    for (const id of ids ?? []) {
      const p = placed.get(id);
      if (p && p.role === role) counts.set(p.tower, (counts.get(p.tower) ?? 0) + 1);
    }
    let best: number | undefined;
    for (const [tower, c] of counts) {
      if (best === undefined || c > (counts.get(best) ?? 0) || (c === counts.get(best) && tower < best)) best = tower;
    }
    return best;
  };
  const ofKind = (kind: string) => [...stickies.values()].filter((n) => (n.data as StickyData).entity.kind === kind);
  for (const n of ofKind('command')) {
    const tower = towerFor(outgoing.get(n.id), 'event');
    if (tower !== undefined) place(n, 'command', tower);
  }
  for (const n of ofKind('readModel')) {
    const tower = towerFor(incoming.get(n.id), 'event');
    if (tower !== undefined) place(n, 'readModel', tower);
  }
  // An automation stands by the stream its command appends to, else by the stream its read model reads.
  for (const n of ofKind('processor')) {
    const tower = towerFor(outgoing.get(n.id), 'command') ?? towerFor(incoming.get(n.id), 'readModel');
    if (tower !== undefined) place(n, 'processor', tower);
  }

  // Floors: one per Board column that holds something here, bottom up. A
  // floor grows a row for each extra item of one kind in one tower, so
  // stacked Board cards do not overlap.
  const slotKey = (p: Placed) => `${namespaceOf(p.node.id)}|${p.box.x}`;
  const slots = [...new Set([...placed.values()].map(slotKey))].sort((a, b) => {
    const [na, xa] = a.split('|');
    const [nb, xb] = b.split('|');
    return na.localeCompare(nb) || Number(xa) - Number(xb);
  });
  const slotOf = new Map(slots.map((key, i) => [key, i]));
  const groups = new Map<string, Placed[]>();
  for (const p of placed.values()) {
    const key = `${p.tower}:${p.role}:${slotOf.get(slotKey(p))}`;
    groups.set(key, [...(groups.get(key) ?? []), p]);
  }
  const rowOf = new Map<string, number>();
  const rows = slots.map(() => 1);
  for (const [key, list] of groups) {
    const slot = Number(key.slice(key.lastIndexOf(':') + 1));
    list.sort((a, b) => a.box.y - b.box.y);
    list.forEach((p, i) => rowOf.set(p.node.id, i));
    rows[slot] = Math.max(rows[slot], list.length);
  }
  const floorY: number[] = [];
  slots.forEach((_, i) => {
    floorY.push(i === 0 ? 0 : floorY[i - 1] + FLOOR + (rows[i - 1] - 1) * SUB_FLOOR);
  });
  const yOf = (p: Placed) => floorY[slotOf.get(slotKey(p)) ?? 0] + (rowOf.get(p.node.id) ?? 0) * SUB_FLOOR;
  // A stream lane with nothing on it gets no tower, and the towers close up in Board lane order.
  const used = [...new Set([...placed.values()].map((p) => p.tower))].sort((a, b) => a - b);
  const column = new Map(used.map((t, i) => [t, i]));
  const axisX = (tower: number) => (column.get(tower) ?? 0) * TOWER_GAP;

  const nodes: SceneNode[] = [];
  const tags: SceneTag[] = [];
  for (const p of placed.values()) {
    const y = yOf(p);
    const x = axisX(p.tower);
    let body: Body;
    if (p.role === 'event') {
      body = { id: p.node.id, center: vec(x, y, 0), shape: { type: 'cylinder', radius: DISC_R, height: DISC_H } };
      // The name hangs under the disc's front edge, so it does not cover the disc.
      tags.push({
        id: `name:${p.node.id}`,
        position: vec(x, y - DISC_H / 2 - 4.5, DISC_R),
        text: p.data.entity.title,
        tone: 'name',
        priority: 3,
        related: [p.data.entity.key],
      });
    } else {
      const a = (ANGLE[p.role] * Math.PI) / 180;
      const center = vec(x + Math.cos(a) * SIDE_R, y, Math.sin(a) * SIDE_R);
      body = { id: p.node.id, center, shape: { type: 'box', width: CARD_W, height: CARD_H, depth: CARD_D } };
    }
    nodes.push({
      id: p.node.id,
      entity: p.data.entity,
      body,
      printed: body.shape.type === 'box',
      occId: p.node.id,
      instanceUid: p.data.instanceUid,
      momentKey: p.data.moment?.key,
      instance: p.data.instance,
      synthetic: p.data.synthetic,
      flag: flagOf(p.data),
    });
  }

  const byId = new Map(nodes.map((n) => [n.id, n]));
  const edges = boardEdges.filter((e) => byId.has(e.source) && byId.has(e.target) && e.source !== e.target);
  const ports = linkPorts(
    edges.map((e) => ({ from: (byId.get(e.source) as SceneNode).body, to: (byId.get(e.target) as SceneNode).body })),
  );
  const links: SceneLink[] = edges.map((e, i) => {
    const { from, to } = ports[i];
    const d = distance(from.point, to.point);
    const mid = scale(add(from.point, to.point), 0.5);
    const a = placed.get(e.source) as Placed;
    const b = placed.get(e.target) as Placed;
    let control: Vec3;
    if (a.tower === b.tower) {
      // Within a tower a link bulges away from the axis, so it does not cut through the discs.
      const radial = vec(mid.x - axisX(a.tower), 0, mid.z);
      const out = Math.hypot(radial.x, radial.z) < 1e-6 ? vec(0, 0, 1) : normalize(radial);
      control = add(mid, scale(out, d * 0.2));
    } else {
      // Across towers a link arcs up and toward the viewer, over the towers in between.
      control = add(mid, vec(0, 12 + d * 0.18, 16));
    }
    return {
      id: e.id,
      from: e.source,
      to: e.target,
      points: [from.point, control, to.point],
      color: strokeOf(e),
      radius: isSeamEdge(e.id) ? 0.8 : 0.55,
      kind: 'edge',
    };
  });

  const decor: SceneDecor[] = [];
  const plates: ScenePlate[] = [];
  const top = floorY.length > 0 ? floorY[floorY.length - 1] + (rows[rows.length - 1] - 1) * SUB_FLOOR : 0;
  for (const t of used) {
    const lane = lanes[t].data as LaneData;
    const x = axisX(t);
    const events = [...placed.values()].filter((p) => p.tower === t && p.role === 'event').map(yOf);
    const low = Math.min(0, ...events) - 8;
    decor.push(
      {
        type: 'shell',
        id: `shell:${lanes[t].id}`,
        center: vec(x, (low + top + 8) / 2, 0),
        radius: DISC_R + 3,
        height: top + 8 - low,
        color: '#fdba74',
        opacity: 0.06,
      },
      {
        type: 'disc',
        id: `base:${lanes[t].id}`,
        center: vec(x, low - 0.6, 0),
        radius: DISC_R + 7,
        height: 1.2,
        color: lane.tint,
      },
    );
    plates.push({
      id: `tower:${lanes[t].id}`,
      center: vec(x, top + 22, 0),
      width: 64,
      height: 18,
      kicker: lane.entity ? 'Event stream' : 'No swimlane',
      title: lane.label,
      sub: lane.entity?.streamId,
      tint: lane.tint,
      border: '#fdba74',
      entity: lane.entity,
    });
  }

  // A faint line behind each floor, labelled with its slice, gives the shared time axis.
  const left = (used.length > 0 ? axisX(used[0]) : 0) - SIDE_R - CARD_W / 2 - 8;
  const right = (used.length > 0 ? axisX(used[used.length - 1]) : 0) + SIDE_R + CARD_W / 2 + 8;
  slots.forEach((key, i) => {
    const y = floorY[i];
    const [ns, slotX] = key.split('|');
    const x = Number(slotX);
    decor.push({
      type: 'line',
      id: `floor:${key}`,
      points: [vec(left, y, -DISC_R - 6), vec(right, y, -DISC_R - 6)],
      color: '#a1a1aa',
      opacity: 0.35,
    });
    const moment = [...placed.values()].find((p) => namespaceOf(p.node.id) === ns && p.box.x === x && p.data.moment)
      ?.data.moment;
    tags.push({
      id: `floor:${key}`,
      position: vec(left - 50, y, -DISC_R - 6),
      text: moment ? moment.title : 'Implicit projection',
      tone: 'ghost',
      priority: 1,
      related: moment ? [moment.key] : undefined,
    });
  });

  const corners = nodes.flatMap((n) => cornersOf(n.body));
  for (const p of plates) corners.push(vec(p.center.x - p.width / 2, p.center.y + p.height / 2, 0));
  corners.push(vec(left - 90, 0, 0), vec(right, 0, 0));
  const all = regionOf(corners, 6);
  const z0 = -DISC_R - 6;
  const z1 = SIDE_R + CARD_D;
  const steps: SceneStep[] = [];
  for (const n of boardNodes) {
    if (n.type !== 'sliceHeader') continue;
    const slice = (n.data as SliceHeaderData).entity;
    const own = [...placed.values()].filter(
      (p) => p.data.moment?.key === slice.key && namespaceOf(p.node.id) === namespaceOf(n.id),
    );
    // A slice that draws nothing here, such as a screen that only reads, is not a stop.
    if (own.length === 0) continue;
    const ys = own.map(yOf);
    const y0 = Math.min(...ys) - FLOOR / 2;
    const y1 = Math.max(...ys) + FLOOR / 2;
    const region: Region = { min: vec(left, y0, z0), max: vec(right, y1, z1) };
    steps.push({
      label: slice.title,
      region,
      highlight: {
        center: vec((left + right) / 2, Math.min(...ys) - DISC_H, (z0 + z1) / 2),
        width: right - left,
        height: z1 - z0,
        facing: 'y',
      },
    });
  }

  return {
    nodes,
    files: [],
    links,
    plates,
    tags,
    decor,
    front: all,
    frontDirection: normalize(vec(0, 0.3, 1)),
    overview: { region: all, direction: normalize(vec(-0.6, 0.55, 1)) },
    steps,
    stepDirection: normalize(vec(-0.3, 0.6, 1)),
    stepNoun: 'Slice',
    legend: [
      'Each tower is one event stream. Its events stack from the bottom up, in storyboard and slice order.',
      'Commands that append sit on the left. Read models that project sit on the right. Automations stand in front.',
      'Floors line up across towers, so one height is one moment.',
    ],
  };
}

function cornersOf(body: Body): Vec3[] {
  const s = body.shape;
  const half =
    s.type === 'box'
      ? vec(s.width / 2, s.height / 2, s.depth / 2)
      : s.type === 'cylinder'
        ? vec(s.radius, s.height / 2, s.radius)
        : vec(s.radius, s.radius, s.radius);
  return [add(body.center, scale(half, -1)), add(body.center, half)];
}

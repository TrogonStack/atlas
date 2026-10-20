// The shared description of a 3D view. A layout (the lane mesh, the stream
// towers) turns a Model into a SceneSpec of plain data, and one renderer
// draws any SceneSpec. The layouts stay pure and testable, and the views
// share picking, selection, camera presets and disposal.

import type { BoardEdgeData } from './layout';
import type { Entity, EntityKind } from './model';
import { add, type Body, dot, normalize, scale, sub, type Vec3, vec } from './ports';

export interface Region {
  min: Vec3;
  max: Vec3;
}

/** One entity occurrence drawn as a solid shape. Cards are thin boxes with the entity printed on the +z face. */
export interface SceneNode {
  /** Unique in the scene. For a Board card this is the Board's occurrence id, so a selection carries across views. */
  id: string;
  entity: Entity;
  body: Body;
  /** Print the entity on the +z face. Shapes without a face get a name tag instead. */
  printed: boolean;
  /** The Board occurrence a click selects, and its moment, as the Board passes them. */
  occId?: string;
  instanceUid?: string;
  momentKey?: string;
  instance?: string;
  synthetic?: boolean;
  /** The validator states the Board marks on a card. */
  flag?: 'pending' | 'error' | 'orphan';
}

/** A code file on the code wall. Files are not model entities, so a click does not select them. */
export interface SceneFile {
  id: string;
  body: Body;
  repo: string;
  path: string;
  symbols: string[];
  /** Keys of the entities that cite this file. */
  citedBy: string[];
}

export interface SceneLink {
  /** For a Board connection this is the Board's edge id, so an edge selection carries across views. */
  id: string;
  from: string;
  to: string;
  /** Bezier control points: 3 for a quadratic curve, 4 for a cubic one. */
  points: Vec3[];
  color: string;
  radius: number;
  kind: 'edge' | 'cite';
  /** Board connection payload; present when `kind` is `edge` so a pick can select the connection. */
  edge?: BoardEdgeData;
}

/** Printed text on a flat plate facing +z: lane labels and slice, storyboard and context headers. */
export interface ScenePlate {
  id: string;
  center: Vec3;
  width: number;
  height: number;
  title: string;
  /** Defaults to the label of `kind`. */
  kicker?: string;
  sub?: string;
  /** Colors the plate like this kind's cards; otherwise `tint` fills it. */
  kind?: EntityKind;
  tint?: string;
  border?: string;
  dashed?: boolean;
  /** Clicking the plate selects this entity. */
  entity?: Entity;
}

/** A DOM label pinned to a point. Names that would overlap a more important one are hidden. */
export interface SceneTag {
  id: string;
  position: Vec3;
  text: string;
  sub?: string;
  tone: 'zone' | 'ghost' | 'file' | 'name';
  priority: number;
  /** Entity keys this tag belongs to: the tag gains priority while one of them is lit. */
  related?: string[];
  /** Hide the tag unless one of `related` is lit. */
  onlyWhenLit?: boolean;
}

export type SceneDecor =
  | {
      type: 'plane';
      id: string;
      center: Vec3;
      width: number;
      height: number;
      color: string;
      opacity: number;
      facing: 'z' | 'y';
    }
  | { type: 'line'; id: string; points: Vec3[]; color: string; opacity: number }
  | { type: 'shell'; id: string; center: Vec3; radius: number; height: number; color: string; opacity: number }
  | { type: 'disc'; id: string; center: Vec3; radius: number; height: number; color: string };

export interface SceneStep {
  label: string;
  region: Region;
  /** The band lit while this step is active. */
  highlight: { center: Vec3; width: number; height: number; facing: 'z' | 'y' };
}

export interface SceneSpec {
  nodes: SceneNode[];
  files: SceneFile[];
  links: SceneLink[];
  plates: ScenePlate[];
  tags: SceneTag[];
  decor: SceneDecor[];
  /** Front frames this region straight on, along `frontDirection`. */
  front: Region;
  frontDirection: Vec3;
  overview: { region: Region; direction: Vec3 };
  /** Step stops in time order, each framed from `stepDirection`. */
  steps: SceneStep[];
  stepDirection: Vec3;
  /** What one step is, for the step control: "Slice". */
  stepNoun: string;
  /** Short legend lines shown in the corner. */
  legend: string[];
}

export function regionOf(points: Vec3[], pad = 0): Region {
  const min = vec(Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY);
  const max = vec(Number.NEGATIVE_INFINITY, Number.NEGATIVE_INFINITY, Number.NEGATIVE_INFINITY);
  for (const p of points) {
    min.x = Math.min(min.x, p.x);
    min.y = Math.min(min.y, p.y);
    min.z = Math.min(min.z, p.z);
    max.x = Math.max(max.x, p.x);
    max.y = Math.max(max.y, p.y);
    max.z = Math.max(max.z, p.z);
  }
  if (points.length === 0) return { min: vec(-pad, -pad, -pad), max: vec(pad, pad, pad) };
  return { min: sub(min, vec(pad, pad, pad)), max: add(max, vec(pad, pad, pad)) };
}

export function unionRegions(regions: Region[]): Region {
  return regionOf(regions.flatMap((r) => [r.min, r.max]));
}

export function regionCenter(r: Region): Vec3 {
  return scale(add(r.min, r.max), 0.5);
}

export interface ScenePose {
  position: Vec3;
  target: Vec3;
}

const cross = (a: Vec3, b: Vec3): Vec3 => vec(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x);

/** The camera's right and up axes when it looks back along `direction`, with world y kept up. */
export function cameraBasis(direction: Vec3): { right: Vec3; up: Vec3; back: Vec3 } {
  const back = normalize(direction);
  const forward = scale(back, -1);
  let right = normalize(cross(forward, vec(0, 1, 0)));
  // Looking straight down or up has no horizon, so the screen's up is -z.
  if (Math.hypot(right.x, right.y, right.z) < 1e-6) right = normalize(cross(forward, vec(0, 0, -1)));
  return { right, up: cross(right, forward), back };
}

/**
 * The closest camera on the ray from the region's center along `direction`
 * that shows every corner of the region. A bounding sphere would leave a
 * wide board small on screen, so each corner is checked against the
 * horizontal and vertical half angles instead.
 */
export function framePose(region: Region, direction: Vec3, aspect: number, fovDeg: number, margin = 1.08): ScenePose {
  const target = regionCenter(region);
  const { right, up, back } = cameraBasis(direction);
  const tanV = Math.tan((fovDeg * Math.PI) / 360);
  const tanH = tanV * Math.max(aspect, 0.1);
  let d = 0;
  for (const x of [region.min.x, region.max.x])
    for (const y of [region.min.y, region.max.y])
      for (const z of [region.min.z, region.max.z]) {
        const q = sub(vec(x, y, z), target);
        const toward = dot(q, back);
        d = Math.max(
          d,
          (margin * Math.abs(dot(q, right))) / tanH + toward,
          (margin * Math.abs(dot(q, up))) / tanV + toward,
          toward + 1,
        );
      }
  return { position: add(target, scale(back, Math.max(d, 20))), target };
}

/**
 * Flow dots for a lit link of this length. A link between neighbours is
 * short because it ends on the rim, and two dots on it would merge into one
 * blot, so short links get none.
 */
export function flowPhases(linkLength: number): number[] {
  if (linkLength < 30) return [];
  if (linkLength < 90) return [0];
  return [0, 0.5];
}

/** The Board's selection, reduced to what a 3D view needs. Board's `Selection` fits this shape. */
export type SceneSelection = { type: 'entity'; key: string; occId?: string } | { type: 'edge'; id: string };

export interface SceneFocus {
  /** Nodes the selection names. */
  selected: Set<string>;
  /** Selected nodes plus both ends of every lit link. */
  lit: Set<string>;
  litLinks: Set<string>;
  /** Entity keys of the selected entities, for plates and tags. */
  keys: Set<string>;
  /** Entity keys of every lit node. */
  litKeys: Set<string>;
  /** Fade unlit nodes: only a node selection does, as on the Board. */
  dimNodes: boolean;
  /** Fade unlit links: any selection does. */
  dimLinks: boolean;
}

/**
 * Mirrors the Board's focus rule: a selected occurrence lights itself, every
 * link touching it, and the node at each link's other end. An entity
 * selected without an occurrence lights all its occurrences. A link
 * selection lights the link and its ends but fades no node.
 */
export function resolveFocus(
  spec: Pick<SceneSpec, 'nodes' | 'files' | 'links'>,
  selections: readonly SceneSelection[],
): SceneFocus {
  const entities = selections.filter((s): s is Extract<SceneSelection, { type: 'entity' }> => s.type === 'entity');
  const occIds = new Set(entities.flatMap((s) => (s.occId ? [s.occId] : [])));
  const bareKeys = new Set(entities.filter((s) => !s.occId).map((s) => s.key));
  const selectedLinks = new Set(selections.flatMap((s) => (s.type === 'edge' ? [s.id] : [])));
  const selected = new Set<string>();
  for (const n of spec.nodes) {
    if (occIds.has(n.occId ?? n.id) || bareKeys.has(n.entity.key)) selected.add(n.id);
  }
  const litLinks = new Set<string>();
  const lit = new Set(selected);
  for (const l of spec.links) {
    if (selected.has(l.from) || selected.has(l.to) || selectedLinks.has(l.id)) {
      litLinks.add(l.id);
      lit.add(l.from);
      lit.add(l.to);
    }
  }
  const keyOf = new Map(spec.nodes.map((n) => [n.id, n.entity.key]));
  const litKeys = new Set<string>();
  for (const id of lit) {
    const key = keyOf.get(id);
    if (key) litKeys.add(key);
  }
  return {
    selected,
    lit,
    litLinks,
    keys: new Set(entities.map((s) => s.key)),
    litKeys,
    dimNodes: selected.size > 0,
    dimLinks: selected.size > 0 || litLinks.size > 0,
  };
}

/**
 * Whether a tag shows under this focus, how much it counts when names
 * compete for room, and whether it belongs to the selection itself.
 */
export function tagState(tag: SceneTag, focus: SceneFocus): { visible: boolean; priority: number; selected: boolean } {
  const lit = (tag.related ?? []).some((k) => focus.litKeys.has(k));
  const selected = (tag.related ?? []).some((k) => focus.keys.has(k));
  if (tag.onlyWhenLit && !lit) return { visible: false, priority: tag.priority, selected: false };
  return { visible: true, priority: tag.priority + (selected ? 20 : lit ? 10 : 0), selected };
}

export interface ScreenBox {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
  priority: number;
}

/**
 * Keeps the labels that fit: the most important first, then each next one
 * only if it does not overlap one already kept. Ties keep the input order,
 * so the result does not flicker between frames.
 */
export function declutter(boxes: readonly ScreenBox[], gap = 3): Set<string> {
  const order = boxes.map((b, i) => ({ b, i })).sort((a, z) => z.b.priority - a.b.priority || a.i - z.i);
  const kept: ScreenBox[] = [];
  for (const { b } of order) {
    const clash = kept.some(
      (k) => b.x < k.x + k.w + gap && k.x < b.x + b.w + gap && b.y < k.y + k.h + gap && k.y < b.y + b.h + gap,
    );
    if (!clash) kept.push(b);
  }
  return new Set(kept.map((k) => k.id));
}

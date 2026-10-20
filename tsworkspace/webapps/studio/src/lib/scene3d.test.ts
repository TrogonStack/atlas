import { describe, expect, it } from 'vitest';
import { buildModel, type Entity } from './model';
import { normalize, sub, type Vec3, vec } from './ports';
import {
  cameraBasis,
  declutter,
  flowPhases,
  framePose,
  type Region,
  regionOf,
  resolveFocus,
  type SceneLink,
  type SceneNode,
  type SceneTag,
  tagState,
} from './scene3d';

const dot = (a: Vec3, b: Vec3) => a.x * b.x + a.y * b.y + a.z * b.z;

function corners(r: Region): Vec3[] {
  const out: Vec3[] = [];
  for (const x of [r.min.x, r.max.x])
    for (const y of [r.min.y, r.max.y]) for (const z of [r.min.z, r.max.z]) out.push(vec(x, y, z));
  return out;
}

/** The largest share of the half field of view any corner uses, across both axes. */
function screenReach(r: Region, direction: Vec3, aspect: number, fov: number): number {
  const pose = framePose(r, direction, aspect, fov);
  const { right, up } = cameraBasis(direction);
  const forward = normalize(sub(pose.target, pose.position));
  const tanV = Math.tan((fov * Math.PI) / 360);
  const tanH = tanV * aspect;
  let reach = 0;
  for (const c of corners(r)) {
    const q = sub(c, pose.position);
    const depth = dot(q, forward);
    expect(depth).toBeGreaterThan(0);
    reach = Math.max(reach, Math.abs(dot(q, right)) / depth / tanH, Math.abs(dot(q, up)) / depth / tanV);
  }
  return reach;
}

describe('framePose', () => {
  const wide: Region = { min: vec(0, -200, -4), max: vec(1600, 0, 4) };
  const tall: Region = { min: vec(0, -900, -4), max: vec(120, 0, 4) };
  const deep: Region = { min: vec(-50, -50, -300), max: vec(50, 50, 20) };

  it.each([
    ['a wide board, straight on', wide, vec(0, 0, 1), 16 / 10],
    ['a wide board, from the side and above', wide, vec(-0.42, 0.3, 1), 16 / 10],
    ['a tall column in a wide window', tall, vec(-0.55, 0.24, 1), 16 / 10],
    ['a deep region in a narrow window', deep, vec(0.4, 0.5, 1), 0.6],
  ])('keeps every corner of %s on screen, with the margin to spare', (_name, region, direction, aspect) => {
    const reach = screenReach(region, direction, aspect, 40);
    expect(reach).toBeLessThanOrEqual(1 / 1.08 + 1e-9);
    // The pose is the closest one that fits, so some corner sits on the margin.
    expect(reach).toBeGreaterThan(1 / 1.08 - 0.02);
  });

  it('looks at the center of the region', () => {
    const pose = framePose(wide, vec(0, 0, 1), 1.6, 40);
    expect(pose.target).toEqual(vec(800, -100, 0));
    expect(pose.position.x).toBeCloseTo(800);
    expect(pose.position.z).toBeGreaterThan(4);
  });
});

describe('cameraBasis', () => {
  it('keeps world y up when the camera looks along a horizon', () => {
    const { right, up } = cameraBasis(vec(0, 0, 1));
    expect(right.x).toBeCloseTo(1);
    expect(up.y).toBeCloseTo(1);
  });

  it('still has a screen up when the camera looks straight down', () => {
    const { right, up } = cameraBasis(vec(0, 1, 0));
    expect(Math.hypot(right.x, right.y, right.z)).toBeCloseTo(1);
    expect(Math.hypot(up.x, up.y, up.z)).toBeCloseTo(1);
  });
});

describe('regionOf', () => {
  it('pads the bounds on every side', () => {
    expect(regionOf([vec(0, 0, 0), vec(10, 5, 2)], 1)).toEqual({ min: vec(-1, -1, -1), max: vec(11, 6, 3) });
  });

  it('gives a small region around the origin when there are no points', () => {
    expect(regionOf([], 2)).toEqual({ min: vec(-2, -2, -2), max: vec(2, 2, 2) });
  });
});

describe('flowPhases', () => {
  it('puts no dot on a short link, one on a medium link and two on a long one', () => {
    expect(flowPhases(12)).toEqual([]);
    expect(flowPhases(29.9)).toEqual([]);
    expect(flowPhases(30)).toEqual([0]);
    expect(flowPhases(89)).toEqual([0]);
    expect(flowPhases(90)).toEqual([0, 0.5]);
  });
});

function entity(slug: string): Entity {
  const model = buildModel([{ command: { id: { namespace: 'shop', slug, version: '1' }, title: slug } }] as never);
  return model.entities[0];
}

describe('resolveFocus', () => {
  const a = entity('a');
  const b = entity('b');
  const c = entity('c');
  const node = (id: string, e: Entity): SceneNode => ({
    id,
    entity: e,
    body: { id, center: vec(0, 0, 0), shape: { type: 'sphere', radius: 1 } },
    printed: false,
    occId: id,
  });
  const link = (id: string, from: string, to: string): SceneLink => ({
    id,
    from,
    to,
    points: [vec(0, 0, 0), vec(1, 1, 1), vec(2, 2, 2)],
    color: '#000000',
    radius: 1,
    kind: 'edge',
  });
  const spec = {
    nodes: [node('a1', a), node('a2', a), node('b1', b), node('c1', c)],
    files: [],
    links: [link('a1-b1', 'a1', 'b1'), link('b1-c1', 'b1', 'c1'), link('a2-c1', 'a2', 'c1')],
  };

  it('lights a selected occurrence, its links and the node at the other end of each', () => {
    const focus = resolveFocus(spec, [{ type: 'entity', key: a.key, occId: 'a1' }]);
    expect([...focus.selected]).toEqual(['a1']);
    expect([...focus.lit].sort()).toEqual(['a1', 'b1']);
    expect([...focus.litLinks]).toEqual(['a1-b1']);
    expect([...focus.litKeys].sort()).toEqual([a.key, b.key].sort());
    expect(focus.dimNodes).toBe(true);
    expect(focus.dimLinks).toBe(true);
  });

  it('lights every occurrence of an entity selected without one', () => {
    const focus = resolveFocus(spec, [{ type: 'entity', key: a.key }]);
    expect([...focus.selected].sort()).toEqual(['a1', 'a2']);
    expect([...focus.litLinks].sort()).toEqual(['a1-b1', 'a2-c1']);
  });

  it('lights a selected link and its ends without fading any node', () => {
    const focus = resolveFocus(spec, [{ type: 'edge', id: 'b1-c1' }]);
    expect(focus.selected.size).toBe(0);
    expect([...focus.lit].sort()).toEqual(['b1', 'c1']);
    expect(focus.dimNodes).toBe(false);
    expect(focus.dimLinks).toBe(true);
  });

  it('fades nothing without a selection', () => {
    const focus = resolveFocus(spec, []);
    expect(focus.lit.size).toBe(0);
    expect(focus.dimNodes).toBe(false);
    expect(focus.dimLinks).toBe(false);
  });
});

describe('tagState', () => {
  const a = entity('a');
  const b = entity('b');
  const tag = (extra: Partial<SceneTag>): SceneTag => ({
    id: 't',
    position: vec(0, 0, 0),
    text: 'name',
    tone: 'file',
    priority: 1,
    ...extra,
  });
  const focus = {
    ...resolveFocus({ nodes: [], files: [], links: [] }, []),
    keys: new Set([a.key]),
    litKeys: new Set([a.key, b.key]),
  };

  it('ranks the names of the selection above the names of its neighbours', () => {
    expect(tagState(tag({ related: [a.key] }), focus)).toEqual({ visible: true, priority: 21, selected: true });
    expect(tagState(tag({ related: [b.key] }), focus)).toEqual({ visible: true, priority: 11, selected: false });
    expect(tagState(tag({ related: ['other'] }), focus)).toEqual({ visible: true, priority: 1, selected: false });
  });

  it('hides a tag that only shows with its entity lit', () => {
    expect(tagState(tag({ related: ['other'], onlyWhenLit: true }), focus).visible).toBe(false);
    expect(tagState(tag({ related: [b.key], onlyWhenLit: true }), focus).visible).toBe(true);
  });
});

describe('declutter', () => {
  const box = (id: string, x: number, priority: number) => ({ id, x, y: 0, w: 40, h: 12, priority });

  it('keeps every label that has room', () => {
    expect([...declutter([box('a', 0, 1), box('b', 50, 1), box('c', 100, 1)])]).toEqual(['a', 'b', 'c']);
  });

  it('keeps the more important of two labels that overlap', () => {
    expect([...declutter([box('low', 0, 1), box('high', 20, 5)])]).toEqual(['high']);
  });

  it('keeps the first of two equal labels, so the choice is stable between frames', () => {
    expect([...declutter([box('first', 0, 1), box('second', 20, 1)])]).toEqual(['first']);
  });

  it('counts the gap as part of a label', () => {
    expect(declutter([box('a', 0, 1), box('b', 42, 1)]).has('b')).toBe(false);
    expect(declutter([box('a', 0, 1), box('b', 42, 1)], 0).has('b')).toBe(true);
  });
});

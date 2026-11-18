import { describe, expect, it } from 'vitest';
import {
  type Body,
  curveLength,
  curvePoint,
  cubicLink,
  distance,
  length,
  linkPorts,
  portOf,
  sub,
  THIN_BOX,
  THIN_DISC,
  vec,
} from './ports';

const box = (id: string, x: number, y: number, z: number, w: number, h: number, d: number): Body => ({
  id,
  center: vec(x, y, z),
  shape: { type: 'box', width: w, height: h, depth: d },
});
// A Board card in world units: wide, short, and thin in z.
const card = (id: string, x: number, y: number) => box(id, x, y, 0, 54, 30, 3);

const close = (a: number, b: number) => expect(a).toBeCloseTo(b, 6);
const closeVec = (a: { x: number; y: number; z: number }, b: { x: number; y: number; z: number }) => {
  close(a.x, b.x);
  close(a.y, b.y);
  close(a.z, b.z);
};

describe('portOf on a box', () => {
  it('ends on the face the ray leaves through, with that face as the normal', () => {
    const cube = box('c', 0, 0, 0, 10, 10, 10);
    const p = portOf(cube, vec(20, 5, 0));
    close(p.point.x, 5);
    close(p.point.y, 1.25);
    close(p.point.z, 0);
    expect(p.normal).toEqual({ x: 1, y: 0, z: 0 });
    expect(p.rim).toBeUndefined();
  });

  it('stays on the surface for every direction', () => {
    const b = box('b', 1, 2, 3, 12, 8, 6);
    for (const t of [vec(9, 0, 0), vec(0, -9, 1), vec(3, 4, 12), vec(-7, 7, -7), vec(1, 2, -9)]) {
      const p = portOf(b, t);
      const q = sub(p.point, b.center);
      const reach = Math.max(Math.abs(q.x) / 6, Math.abs(q.y) / 4, Math.abs(q.z) / 3);
      expect(reach).toBeCloseTo(1, 6);
    }
  });

  it('puts the port on the rim of a thin card, not on its printed face', () => {
    const c = card('a', 0, 0);
    // The far end sits in front of the card and to the right: a center ray
    // would hit the face, the rim rule moves the port to the right edge.
    const p = portOf(c, vec(100, 0, 40));
    close(p.point.x, 27);
    close(p.point.z, 0);
    expect(p.normal).toEqual({ x: 1, y: 0, z: 0 });
    expect(p.rim).toEqual({ axis: 'y', side: 'x+', half: 15 });
  });

  it('keeps the face when the far end is straight in front of the card', () => {
    const p = portOf(card('a', 0, 0), vec(0, 0, 50));
    close(p.point.z, 1.5);
    expect(p.normal).toEqual({ x: 0, y: 0, z: 1 });
    expect(p.rim).toBeUndefined();
  });

  it('treats a box as a card only below the thin ratio', () => {
    const thick = box('t', 0, 0, 0, 20, 20, THIN_BOX * 20 + 0.01);
    expect(portOf(thick, vec(30, 0, 30)).rim).toBeUndefined();
    const thin = box('t', 0, 0, 0, 20, 20, THIN_BOX * 20 - 0.01);
    expect(portOf(thin, vec(30, 0, 30)).rim?.side).toBe('x+');
  });

  it('names the bottom rim and runs its tangent along x', () => {
    const p = portOf(card('a', 0, 0), vec(10, -200, 0));
    close(p.point.y, -15);
    expect(p.rim).toEqual({ axis: 'x', side: 'y-', half: 27 });
  });
});

describe('portOf on round shapes', () => {
  it('ends a sphere link at the radius along the ray', () => {
    const s: Body = { id: 's', center: vec(1, 1, 1), shape: { type: 'sphere', radius: 4 } };
    const p = portOf(s, vec(1, 11, 1));
    expect(p.point).toEqual({ x: 1, y: 5, z: 1 });
    expect(p.normal).toEqual({ x: 0, y: 1, z: 0 });
    const q = portOf(s, vec(7, -3, 9));
    close(distance(q.point, s.center), 4);
  });

  it('ends an octahedron link on a face, with the face normal', () => {
    const o: Body = { id: 'o', center: vec(0, 0, 0), shape: { type: 'octahedron', radius: 9 } };
    const p = portOf(o, vec(5, 5, 5));
    close(Math.abs(p.point.x) + Math.abs(p.point.y) + Math.abs(p.point.z), 9);
    close(p.normal.x, 1 / Math.sqrt(3));
    const tip = portOf(o, vec(0, 20, 0));
    expect(tip.point).toEqual({ x: 0, y: 9, z: 0 });
  });

  it('ends a tall cylinder link on its side or its cap', () => {
    const c: Body = { id: 'c', center: vec(0, 0, 0), shape: { type: 'cylinder', radius: 5, height: 40 } };
    const side = portOf(c, vec(10, 2, 0));
    close(Math.hypot(side.point.x, side.point.z), 5);
    expect(side.normal).toEqual({ x: 1, y: 0, z: 0 });
    const cap = portOf(c, vec(1, 50, 0));
    close(cap.point.y, 20);
    expect(cap.normal).toEqual({ x: 0, y: 1, z: 0 });
  });

  it('moves a slanted link on a thin disc to the rim', () => {
    const disc: Body = { id: 'd', center: vec(0, 10, 0), shape: { type: 'cylinder', radius: 18, height: 3.4 } };
    expect(3.4).toBeLessThan(THIN_DISC * 18);
    const p = portOf(disc, vec(30, 40, 0));
    close(p.point.y, 10);
    close(p.point.x, 18);
    expect(p.normal).toEqual({ x: 1, y: 0, z: 0 });
    // Straight up still uses the top face.
    const up = portOf(disc, vec(0, 40, 0));
    close(up.point.y, 11.7);
  });

  it('returns the center with no normal when the far end is the center', () => {
    const p = portOf(card('a', 3, 4), vec(3, 4, 0));
    expect(p.point).toEqual({ x: 3, y: 4, z: 0 });
    expect(length(p.normal)).toBe(0);
  });
});

describe('linkPorts', () => {
  it('spreads links that share a rim along it, in the order of their far ends', () => {
    const cmd = card('cmd', 0, 0);
    const left = card('left', -60, -80);
    const middle = card('middle', 5, -80);
    const right = card('right', 70, -80);
    // Listed out of order on purpose: the spread follows the far ends, not the input.
    const ports = linkPorts([
      { from: cmd, to: right },
      { from: cmd, to: left },
      { from: cmd, to: middle },
    ]);
    const xs = ports.map((p) => p.from.point.x);
    expect(ports.every((p) => p.from.rim?.side === 'y-')).toBe(true);
    expect(xs[1]).toBeLessThan(xs[2]);
    expect(xs[2]).toBeLessThan(xs[0]);
    // Centered on the rim and no wider than 8 apart.
    close(xs[0] + xs[1] + xs[2], 0);
    close(xs[0] - xs[2], 8);
    for (const p of ports) close(p.from.point.y, -15);
  });

  it('keeps a crowded rim to its middle 70%', () => {
    const cmd = card('cmd', 0, 0);
    const far = Array.from({ length: 12 }, (_, i) => card(`e${i}`, i * 10 - 60, -100));
    const ports = linkPorts(far.map((to) => ({ from: cmd, to })));
    const xs = ports.map((p) => p.from.point.x);
    expect(Math.max(...xs) - Math.min(...xs)).toBeCloseTo(0.7 * 54, 6);
  });

  it('spreads both ends of a link and leaves a lone rim port where it lands', () => {
    const a = card('a', 0, 0);
    const b = card('b', 0, -100);
    const c = card('c', 60, 0);
    const [ab, cb] = linkPorts([
      { from: a, to: b },
      { from: c, to: b },
    ]);
    close(ab.from.point.x, 0);
    // Both links reach b's top rim, so those two ends spread.
    expect(ab.to.point.x).toBeLessThan(cb.to.point.x);
    expect(ab.to.rim?.side).toBe('y+');
  });

  it('uses the aim point to choose a rim', () => {
    const a = card('a', 0, 0);
    const wall = box('f', 0, 0, -160, 10, 10, 6);
    const [straight] = linkPorts([{ from: a, to: wall }]);
    expect(straight.from.normal).toEqual({ x: 0, y: 0, z: -1 });
    const [aimed] = linkPorts([{ from: a, to: wall, fromToward: vec(0, -200, 0) }]);
    expect(aimed.from.rim?.side).toBe('y-');
  });
});

describe('curves', () => {
  it('runs a cubic link from port to port through the lift', () => {
    const [p] = linkPorts([{ from: card('a', 0, 0), to: card('b', 0, -100) }]);
    const points = cubicLink(p.from, p.to, 20, vec(0, 0, 25));
    closeVec(curvePoint(points, 0), p.from.point);
    closeVec(curvePoint(points, 1), p.to.point);
    expect(curvePoint(points, 0.5).z).toBeCloseTo(0.75 * 25, 6);
  });

  it('evaluates a quadratic curve and measures a straight one exactly', () => {
    const q = [vec(0, 0, 0), vec(5, 10, 0), vec(10, 0, 0)];
    closeVec(curvePoint(q, 0.5), vec(5, 5, 0));
    close(curveLength([vec(0, 0, 0), vec(3, 0, 0), vec(6, 0, 0), vec(9, 0, 0)]), 9);
    expect(curveLength(q)).toBeGreaterThan(10);
  });
});

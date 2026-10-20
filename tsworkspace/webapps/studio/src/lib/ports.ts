// Edge ports for the 3D views: the point where a link meets the shape it
// connects. A link that ends at a shape's center is hidden inside the shape,
// so every link ends on the surface instead. A flat card or disc takes its
// links on the rim, because a link on the printed face would cover the text.
// Links that share one rim of a card spread along it in the order of their
// far ends, so they leave side by side and do not cross at one point.
//
// Plain vectors only: the layouts that call this are pure and tested without
// a renderer.

export interface Vec3 {
  x: number;
  y: number;
  z: number;
}

export const vec = (x: number, y: number, z: number): Vec3 => ({ x, y, z });
export const add = (a: Vec3, b: Vec3): Vec3 => ({ x: a.x + b.x, y: a.y + b.y, z: a.z + b.z });
export const sub = (a: Vec3, b: Vec3): Vec3 => ({ x: a.x - b.x, y: a.y - b.y, z: a.z - b.z });
export const scale = (a: Vec3, k: number): Vec3 => ({ x: a.x * k, y: a.y * k, z: a.z * k });
export const dot = (a: Vec3, b: Vec3): number => a.x * b.x + a.y * b.y + a.z * b.z;
export const length = (a: Vec3): number => Math.hypot(a.x, a.y, a.z);
export const distance = (a: Vec3, b: Vec3): number => length(sub(a, b));
export const lerp = (a: Vec3, b: Vec3, t: number): Vec3 => add(a, scale(sub(b, a), t));
export function normalize(a: Vec3): Vec3 {
  const l = length(a);
  return l > EPS ? scale(a, 1 / l) : vec(0, 0, 0);
}

const EPS = 1e-6;
type Axis = 'x' | 'y' | 'z';
const AXES: readonly Axis[] = ['x', 'y', 'z'];

// A box is a card when its smallest side is under this share of the next one.
export const THIN_BOX = 0.4;
// A cylinder is a disc when its height is under this share of its radius.
export const THIN_DISC = 0.8;

// Cylinders stand on the y axis, the way three.js builds them.
export type Shape =
  | { type: 'box'; width: number; height: number; depth: number }
  | { type: 'sphere'; radius: number }
  | { type: 'octahedron'; radius: number }
  | { type: 'cylinder'; radius: number; height: number };

export interface Body {
  /** Groups the ports that share a rim, so it must be unique per shape. */
  id: string;
  center: Vec3;
  shape: Shape;
}

export interface Port {
  point: Vec3;
  /** Outward surface normal at `point`; zero when the link has no direction. */
  normal: Vec3;
  /** Set when the port sits on a card's rim: the rim's direction, which rim, and half its length. */
  rim?: { axis: Axis; side: string; half: number };
}

/** The port where a link from `body` toward `toward` leaves the surface. */
export function portOf(body: Body, toward: Vec3): Port {
  const c = body.center;
  const v = sub(toward, c);
  if (length(v) < EPS) return { point: c, normal: vec(0, 0, 0) };
  const s = body.shape;
  if (s.type === 'box') return boxPort(c, [s.width, s.height, s.depth], v);
  if (s.type === 'cylinder') return cylinderPort(c, s.radius, s.height, normalize(v));
  const n = normalize(v);
  if (s.type === 'sphere') return { point: add(c, scale(n, s.radius)), normal: n };
  // An octahedron is |x| + |y| + |z| = r, so the ray reaches it at r / (|nx| + |ny| + |nz|).
  const t = s.radius / (Math.abs(n.x) + Math.abs(n.y) + Math.abs(n.z));
  return { point: add(c, scale(n, t)), normal: normalize(vec(Math.sign(n.x), Math.sign(n.y), Math.sign(n.z))) };
}

function boxPort(c: Vec3, dims: [number, number, number], v: Vec3): Port {
  const sorted = [...dims].sort((a, b) => a - b);
  const thin = sorted[0] < THIN_BOX * sorted[1] ? dims.indexOf(sorted[0]) : -1;
  const d = [v.x, v.y, v.z];
  // A link toward a card's side leaves from the rim, not from the face. A
  // link straight through the face keeps the face, so it is never lost.
  if (thin >= 0 && Math.hypot(...d.map((x, i) => (i === thin ? 0 : x))) > 1e-3) d[thin] = 0;
  let t = Number.POSITIVE_INFINITY;
  let axis = -1;
  for (let i = 0; i < 3; i++) {
    if (Math.abs(d[i]) < EPS) continue;
    const ti = dims[i] / 2 / Math.abs(d[i]);
    if (ti < t) {
      t = ti;
      axis = i;
    }
  }
  const point = vec(c.x + d[0] * t, c.y + d[1] * t, c.z + d[2] * t);
  const n = [0, 0, 0];
  n[axis] = d[axis] > 0 ? 1 : -1;
  const port: Port = { point, normal: vec(n[0], n[1], n[2]) };
  if (thin >= 0 && axis !== thin) {
    const tangent = 3 - thin - axis;
    port.rim = { axis: AXES[tangent], side: `${AXES[axis]}${d[axis] > 0 ? '+' : '-'}`, half: dims[tangent] / 2 };
  }
  return port;
}

function cylinderPort(c: Vec3, radius: number, height: number, n: Vec3): Port {
  let d = n;
  // A disc takes its links on the rim unless the link goes straight up or down.
  if (height < THIN_DISC * radius && Math.hypot(n.x, n.z) > 1e-3) d = normalize(vec(n.x, 0, n.z));
  const radial = Math.hypot(d.x, d.z);
  const tSide = radial > EPS ? radius / radial : Number.POSITIVE_INFINITY;
  const tCap = Math.abs(d.y) > EPS ? height / 2 / Math.abs(d.y) : Number.POSITIVE_INFINITY;
  const point = add(c, scale(d, Math.min(tSide, tCap)));
  const normal = tSide <= tCap ? normalize(vec(d.x, 0, d.z)) : vec(0, Math.sign(d.y), 0);
  return { point, normal };
}

export interface PortLink {
  from: Body;
  to: Body;
  /** Where the `from` end aims instead of the `to` center, to pick a rim. */
  fromToward?: Vec3;
  toToward?: Vec3;
}

/**
 * Ports for a set of links, with the ports that share one rim of one card
 * spread along that rim. The spread follows the far ends' order along the
 * rim, so two links that leave side by side never swap places and cross.
 * Links are spaced at most `maxStep` apart and keep to the middle 70% of the
 * rim, so no port lands on a corner.
 */
export function linkPorts(links: PortLink[], maxStep = 8): { from: Port; to: Port }[] {
  const out = links.map((l) => ({
    from: portOf(l.from, l.fromToward ?? l.to.center),
    to: portOf(l.to, l.toToward ?? l.from.center),
  }));
  const groups = new Map<string, { port: Port; body: Body; far: number; order: number }[]>();
  out.forEach((ends, i) => {
    const pairs: [Port, Body, Vec3][] = [
      [ends.from, links[i].from, links[i].to.center],
      [ends.to, links[i].to, links[i].from.center],
    ];
    for (const [port, body, far] of pairs) {
      if (!port.rim) continue;
      const key = `${body.id}\0${port.rim.side}`;
      const list = groups.get(key) ?? [];
      list.push({ port, body, far: far[port.rim.axis], order: list.length });
      groups.set(key, list);
    }
  });
  for (const list of groups.values()) {
    if (list.length < 2) continue;
    list.sort((a, b) => a.far - b.far || a.order - b.order);
    const rim = list[0].port.rim;
    if (!rim) continue;
    const step = Math.min(maxStep, (1.4 * rim.half) / (list.length - 1));
    list.forEach((entry, k) => {
      const offset = (k - (list.length - 1) / 2) * step;
      entry.port.point = { ...entry.port.point, [rim.axis]: entry.body.center[rim.axis] + offset };
    });
  }
  return out;
}

/**
 * A cubic curve that leaves each port along its normal for `reach`, then
 * bends through `lift`. The lift moves both inner control points, so a link
 * between two cards on one wall arcs off the wall and does not cut across
 * the cards in between.
 */
export function cubicLink(a: Port, b: Port, reach: number, lift: Vec3): Vec3[] {
  return [
    a.point,
    add(add(a.point, scale(a.normal, reach)), lift),
    add(add(b.point, scale(b.normal, reach)), lift),
    b.point,
  ];
}

/** A point on a quadratic (3 points) or cubic (4 points) Bezier curve. */
export function curvePoint(points: Vec3[], t: number): Vec3 {
  const u = 1 - t;
  if (points.length === 3) {
    const [p0, p1, p2] = points;
    return add(add(scale(p0, u * u), scale(p1, 2 * u * t)), scale(p2, t * t));
  }
  if (points.length === 4) {
    const [p0, p1, p2, p3] = points;
    return add(
      add(scale(p0, u * u * u), scale(p1, 3 * u * u * t)),
      add(scale(p2, 3 * u * t * t), scale(p3, t * t * t)),
    );
  }
  return points.length > 0 ? lerp(points[0], points[points.length - 1], t) : vec(0, 0, 0);
}

/** Curve length from `samples` straight pieces: close enough to choose how many flow dots fit. */
export function curveLength(points: Vec3[], samples = 24): number {
  let total = 0;
  let prev = curvePoint(points, 0);
  for (let i = 1; i <= samples; i++) {
    const next = curvePoint(points, i / samples);
    total += distance(prev, next);
    prev = next;
  }
  return total;
}

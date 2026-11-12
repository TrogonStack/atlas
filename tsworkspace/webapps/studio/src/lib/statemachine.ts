// The state machine view: one stream's lifecycle (proto Swimlane.transitions)
// drawn as a diagram. After an event lands on the stream, exactly one of its
// `next` events follows.
//
// Decision #24 (TIME ONLY POINTS FORWARD) binds this view: a transition back
// to an earlier state is drawn as a forward re-entry stub, never as an arrow
// into the past. Columns run left to right, and every arrow ends in a later
// column than it starts.
//
// The table can be partial, and its gaps mean different things (the
// server's lifecycle conformance check): an event with no row puts no limit
// on what follows it, while a row with an empty `next` is a final state. The
// proto names no start state, so the layout claims none. Events that no
// transition leads to just sit first.

import { asId, type Entity, type EntityId, entityKey, type Model, slugKey, versionOf } from './model';

export const STATE_W = 176;
export const STATE_H = 48;
// A stub names the state it returns to, so it gets the width and two lines of a state.
export const STUB_W = STATE_W;
export const STUB_H = 40;
const COL_GAP = 64;
const ROW_GAP = 16;
// Long arrows pass through thin waypoints, one per column they cross, so
// they route between states instead of through them.
const WAY_H = 6;
const WAY_GAP = 8;
const PAD = 24;
// Room for the marks drawn past a state's right edge.
const PAD_RIGHT = 40;
// Where an arrow stops short of its target, so the arrowhead shows.
export const ARROW_GAP = 6;

/** `row`: the table lists what may follow. `open`: no row, anything may follow. `final`: an empty row, nothing may. */
export type StateKind = 'row' | 'open' | 'final';
/** `missing`: no event in the model has this id. `foreign`: the event belongs to another stream. */
export type StateProblem = 'missing' | 'foreign';

export interface Point {
  x: number;
  y: number;
}

export interface LifecycleState {
  /** slugKey of the event: a state is the moment after that event lands. */
  id: string;
  ref: EntityId;
  entity?: Entity;
  title: string;
  kind: StateKind;
  problem?: StateProblem;
  column: number;
  /** Center. */
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface LifecycleStub {
  id: string;
  /** The state the lifecycle goes back to. */
  returnsTo: string;
  /** The state the return leaves from. */
  from: string;
  title: string;
  column: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface LifecycleArrow {
  id: string;
  from: string;
  /** A state id, or a stub id for a return. */
  to: string;
  /** The state this arrow leads to; for a return, the earlier state. */
  target: string;
  guard: string;
  /** Position in the source row's `next` list. */
  order: number;
  reentry: boolean;
  points: Point[];
}

export interface Lifecycle {
  lane: Entity;
  states: LifecycleState[];
  stubs: LifecycleStub[];
  arrows: LifecycleArrow[];
  /** Events on this stream that no transition names. */
  untracked: Entity[];
  columns: number;
  width: number;
  height: number;
}

interface Row {
  after: EntityId;
  next: { ref: EntityId; guard: string }[];
}

function rawRows(lane: Entity): Row[] {
  const raw = Array.isArray(lane.raw.transitions) ? (lane.raw.transitions as Record<string, unknown>[]) : [];
  const rows: Row[] = [];
  for (const t of raw) {
    const after = (t.after as { id?: unknown } | undefined)?.id;
    if (!after) continue;
    const next: Row['next'] = [];
    for (const n of Array.isArray(t.next) ? (t.next as Record<string, unknown>[]) : []) {
      const id = (n.event as { id?: unknown } | undefined)?.id;
      if (id) next.push({ ref: asId(id), guard: String(n.doc ?? '') });
    }
    rows.push({ after: asId(after), next });
  }
  return rows;
}

function latestBySlug(entities: readonly Entity[]): Map<string, Entity> {
  const out = new Map<string, Entity>();
  for (const e of entities) {
    const k = slugKey(e.kind, e.id);
    const prev = out.get(k);
    if (!prev || versionOf(e.id) > versionOf(prev.id)) out.set(k, e);
  }
  return out;
}

/** The streams that declare a lifecycle, newest version of each, in lane order. */
export function lifecycleLanes(model: Model): Entity[] {
  return [...latestBySlug(model.swimlanes).values()]
    .filter((lane) => rawRows(lane).length > 0)
    .sort((a, b) => (a.order ?? 0) - (b.order ?? 0) || a.title.localeCompare(b.title));
}

/** True when some stream declares transitions: the State machine tab then has something to draw. */
export function hasLifecycles(model: Model): boolean {
  return lifecycleLanes(model).length > 0;
}

interface LNode {
  id: string;
  type: 'state' | 'stub' | 'way';
  column: number;
  w: number;
  h: number;
  x: number;
  y: number;
  discovered: number;
}

interface LEdge {
  id: string;
  from: string;
  to: string;
  target: string;
  guard: string;
  order: number;
  reentry: boolean;
  chain: string[];
}

export function buildLifecycle(model: Model, lane: Entity): Lifecycle {
  const laneKey = slugKey('swimlane', lane.id);
  const events = latestBySlug(model.byKind.get('event') ?? []);
  const resolve = (ref: EntityId): Entity | undefined =>
    model.byKey.get(entityKey('event', ref)) ?? events.get(slugKey('event', ref));

  // Rows by state, in table order. Two rows for one event are read as one.
  const order: string[] = [];
  const refs = new Map<string, EntityId>();
  const rows = new Map<string, { to: string; guard: string }[]>();
  const see = (ref: EntityId) => {
    const k = slugKey('event', ref);
    if (!refs.has(k)) {
      refs.set(k, ref);
      order.push(k);
    }
    return k;
  };
  for (const r of rawRows(lane)) {
    const from = see(r.after);
    const list = rows.get(from) ?? [];
    rows.set(from, list);
    for (const n of r.next) {
      const to = see(n.ref);
      if (!list.some((x) => x.to === to)) list.push({ to, guard: n.guard });
    }
  }

  const nodes = new Map<string, LNode>();
  const states = new Map<string, Omit<LifecycleState, 'x' | 'y' | 'column'>>();
  for (const k of order) {
    const ref = refs.get(k) ?? asId(undefined);
    const entity = resolve(ref);
    const row = rows.get(k);
    const onLane = entity?.swimlane ? slugKey('swimlane', entity.swimlane) === laneKey : false;
    states.set(k, {
      id: k,
      ref,
      entity,
      title: entity?.title || ref.slug,
      kind: !row ? 'open' : row.length === 0 ? 'final' : 'row',
      problem: !entity ? 'missing' : onLane ? undefined : 'foreign',
      w: STATE_W,
      h: STATE_H,
    });
  }

  // Depth-first from the states nothing leads to, then from any state left
  // (a lifecycle that is one cycle has no such state). An arrow into a state
  // still on the stack, a self-loop included, points back in time.
  const incoming = new Map(order.map((k) => [k, 0]));
  for (const [from, list] of rows)
    for (const n of list) if (n.to !== from) incoming.set(n.to, (incoming.get(n.to) ?? 0) + 1);
  const color = new Map<string, 1 | 2>();
  const discovered = new Map<string, number>();
  const finished: string[] = [];
  const forward: Omit<LEdge, 'id' | 'chain'>[] = [];
  const back: Omit<LEdge, 'id' | 'chain'>[] = [];
  const visit = (v: string) => {
    color.set(v, 1);
    discovered.set(v, discovered.size);
    (rows.get(v) ?? []).forEach((n, i) => {
      const edge = { from: v, to: n.to, target: n.to, guard: n.guard, order: i, reentry: false };
      if (color.get(n.to) === 1) back.push({ ...edge, reentry: true });
      else {
        forward.push(edge);
        if (!color.has(n.to)) visit(n.to);
      }
    });
    color.set(v, 2);
    finished.push(v);
  };
  for (const k of order) if (incoming.get(k) === 0 && !color.has(k)) visit(k);
  for (const k of order) if (!color.has(k)) visit(k);

  // Columns: the longest forward path into each state. A state nothing leads
  // to then moves right, to just before its first successor.
  const column = new Map(order.map((k) => [k, 0]));
  const outs = new Map(order.map((k) => [k, forward.filter((e) => e.from === k)]));
  const topo = [...finished].reverse();
  for (const v of topo)
    for (const e of outs.get(v) ?? []) column.set(e.to, Math.max(column.get(e.to) ?? 0, (column.get(v) ?? 0) + 1));
  for (const v of finished) {
    const o = outs.get(v) ?? [];
    if (incoming.get(v) === 0 && o.length > 0) column.set(v, Math.min(...o.map((e) => column.get(e.to) ?? 0)) - 1);
  }

  for (const k of order)
    nodes.set(k, {
      id: k,
      type: 'state',
      column: column.get(k) ?? 0,
      w: STATE_W,
      h: STATE_H,
      x: 0,
      y: 0,
      discovered: discovered.get(k) ?? 0,
    });

  // One stub per return, in the column after the state it leaves from.
  const edges: LEdge[] = forward.map((e, i) => ({ ...e, id: `t${i}:${e.from}>${e.to}`, chain: [] }));
  const stubs = new Map<string, LifecycleStub>();
  back.forEach((e, i) => {
    const id = `stub:${e.from}>${e.target}`;
    const fromNode = nodes.get(e.from);
    const col = (fromNode?.column ?? 0) + 1;
    nodes.set(id, {
      id,
      type: 'stub',
      column: col,
      w: STUB_W,
      h: STUB_H,
      x: 0,
      y: 0,
      discovered: (fromNode?.discovered ?? 0) + 0.5,
    });
    stubs.set(id, {
      id,
      returnsTo: e.target,
      from: e.from,
      title: states.get(e.target)?.title ?? e.target,
      column: col,
      x: 0,
      y: 0,
      w: STUB_W,
      h: STUB_H,
    });
    edges.push({ ...e, id: `r${i}:${e.from}>${e.target}`, to: id, chain: [] });
  });

  // Waypoints for arrows that cross columns.
  const segs: [string, string][] = [];
  for (const e of edges) {
    const a = nodes.get(e.from)?.column ?? 0;
    const b = nodes.get(e.to)?.column ?? 0;
    const from = nodes.get(e.from);
    let prev = e.from;
    e.chain = [e.from];
    for (let c = a + 1; c < b; c++) {
      const id = `way:${e.id}:${c}`;
      nodes.set(id, {
        id,
        type: 'way',
        column: c,
        w: 0,
        h: WAY_H,
        x: 0,
        y: 0,
        discovered: (from?.discovered ?? 0) + 0.25,
      });
      segs.push([prev, id]);
      e.chain.push(id);
      prev = id;
    }
    segs.push([prev, e.to]);
    e.chain.push(e.to);
  }
  const preds = new Map<string, string[]>();
  const succs = new Map<string, string[]>();
  for (const id of nodes.keys()) {
    preds.set(id, []);
    succs.set(id, []);
  }
  for (const [u, v] of segs) {
    succs.get(u)?.push(v);
    preds.get(v)?.push(u);
  }

  // Order inside each column: barycenter sweeps, keeping the order with the
  // fewest crossings.
  const nCols = nodes.size ? Math.max(...[...nodes.values()].map((n) => n.column)) + 1 : 0;
  let cols: string[][] = Array.from({ length: nCols }, () => []);
  for (const n of nodes.values()) cols[n.column].push(n.id);
  const disc = (id: string) => nodes.get(id)?.discovered ?? 0;
  for (const col of cols) col.sort((a, b) => disc(a) - disc(b));
  const pos = new Map<string, number>();
  const index = () => cols.forEach((col) => col.forEach((id, i) => pos.set(id, i)));
  const at = (id: string) => pos.get(id) ?? 0;
  const segsFrom = cols.map((_, c) => segs.filter(([u]) => nodes.get(u)?.column === c));
  const crossings = () => {
    let count = 0;
    for (const s of segsFrom)
      for (let i = 0; i < s.length; i++)
        for (let j = i + 1; j < s.length; j++)
          if ((at(s[i][0]) - at(s[j][0])) * (at(s[i][1]) - at(s[j][1])) < 0) count++;
    return count;
  };
  index();
  let best = crossings();
  let bestCols = cols.map((c) => [...c]);
  for (let pass = 0; pass < 16 && best > 0; pass++) {
    const down = pass % 2 === 0;
    const seq = [...Array(nCols).keys()];
    for (const c of down ? seq.slice(1) : seq.reverse().slice(1)) {
      const nb = down ? preds : succs;
      const center = new Map(
        cols[c].map((id) => {
          const ns = nb.get(id) ?? [];
          return [id, ns.length ? ns.reduce((s, x) => s + at(x), 0) / ns.length : at(id)];
        }),
      );
      cols[c].sort((a, b) => (center.get(a) ?? 0) - (center.get(b) ?? 0) || at(a) - at(b));
      cols[c].forEach((id, i) => pos.set(id, i));
    }
    const count = crossings();
    if (count < best) {
      best = count;
      bestCols = cols.map((col) => [...col]);
    }
  }
  cols = bestCols;
  index();

  // Coordinates: each column stacked, then pulled toward its neighbors.
  const gap = (a: string, b: string) =>
    nodes.get(a)?.type === 'way' || nodes.get(b)?.type === 'way' ? WAY_GAP : ROW_GAP;
  const node = (id: string): LNode => nodes.get(id) as LNode;
  cols.forEach((col, c) => {
    let y = 0;
    col.forEach((id, i) => {
      const n = node(id);
      if (i) y += gap(col[i - 1], id);
      n.x = c * (STATE_W + COL_GAP) + STATE_W / 2;
      n.y = y + n.h / 2;
      y += n.h;
    });
    for (const id of col) node(id).y -= y / 2;
  });
  const place = (col: string[], want: number[]) => {
    const ys: number[] = [];
    col.forEach((id, i) => {
      const prev = col[i - 1];
      ys[i] =
        prev === undefined ? want[i] : Math.max(want[i], ys[i - 1] + node(prev).h / 2 + gap(prev, id) + node(id).h / 2);
    });
    const shift = want.reduce((s, w, i) => s + (w - ys[i]), 0) / col.length;
    col.forEach((id, i) => {
      node(id).y = ys[i] + shift;
    });
  };
  for (let pass = 0; pass < 8; pass++) {
    const down = pass % 2 === 0;
    const seq = [...Array(nCols).keys()];
    for (const c of down ? seq : seq.reverse()) {
      const nb = down ? preds : succs;
      const want = cols[c].map((id) => {
        const ys = (nb.get(id) ?? []).map((x) => node(x).y);
        return ys.length ? ys.reduce((a, b) => a + b, 0) / ys.length : node(id).y;
      });
      if (cols[c].length) place(cols[c], want);
    }
  }

  // Bounding box, then shift everything so it starts at (PAD, PAD).
  const all = [...nodes.values()];
  const x0 = all.length ? Math.min(...all.map((n) => n.x - n.w / 2)) : 0;
  const y0 = all.length ? Math.min(...all.map((n) => n.y - n.h / 2)) : 0;
  const x1 = all.length ? Math.max(...all.map((n) => n.x + n.w / 2)) : 0;
  const y1 = all.length ? Math.max(...all.map((n) => n.y + n.h / 2)) : 0;
  for (const n of all) {
    n.x += PAD - x0;
    n.y += PAD - y0;
  }

  // Ports: a state's arrows spread along its edge in the order of their far ends.
  const outPort = new Map<string, number>();
  const inPort = new Map<string, number>();
  const spread = (id: string, list: LEdge[], far: (e: LEdge) => number, into: Map<string, number>) => {
    const n = node(id);
    const sorted = [...list].sort((a, b) => far(a) - far(b));
    const span = Math.min(n.h * 0.6, (sorted.length - 1) * 7);
    sorted.forEach((e, i) =>
      into.set(`${id}|${e.id}`, n.y - span / 2 + (sorted.length > 1 ? (span * i) / (sorted.length - 1) : span / 2)),
    );
  };
  for (const n of all) {
    if (n.type === 'way') continue;
    spread(
      n.id,
      edges.filter((e) => e.from === n.id),
      (e) => node(e.chain[1]).y,
      outPort,
    );
    spread(
      n.id,
      edges.filter((e) => e.to === n.id),
      (e) => node(e.chain[e.chain.length - 2]).y,
      inPort,
    );
  }

  const arrows: LifecycleArrow[] = edges.map((e) => ({
    id: e.id,
    from: e.from,
    to: e.to,
    target: e.target,
    guard: e.guard,
    order: e.order,
    reentry: e.reentry,
    points: e.chain.map((id, i) => {
      const n = node(id);
      if (i === 0) return { x: n.x + n.w / 2, y: outPort.get(`${id}|${e.id}`) ?? n.y };
      if (i === e.chain.length - 1) return { x: n.x - n.w / 2 - ARROW_GAP, y: inPort.get(`${id}|${e.id}`) ?? n.y };
      return { x: n.x, y: n.y };
    }),
  }));

  const untracked = [...events.values()]
    .filter((e) => e.swimlane && slugKey('swimlane', e.swimlane) === laneKey && !states.has(slugKey('event', e.id)))
    .sort((a, b) => a.title.localeCompare(b.title));

  return {
    lane,
    states: order.map((k) => {
      const s = states.get(k) as Omit<LifecycleState, 'x' | 'y' | 'column'>;
      const n = node(k);
      return { ...s, column: n.column, x: n.x, y: n.y };
    }),
    stubs: [...stubs.values()].map((s) => {
      const n = node(s.id);
      return { ...s, x: n.x, y: n.y };
    }),
    arrows,
    untracked,
    columns: nCols,
    width: all.length ? x1 - x0 + PAD + PAD_RIGHT : 0,
    height: all.length ? y1 - y0 + 2 * PAD : 0,
  };
}

/** A smooth path through an arrow's points, level at each end like the Board's edges. */
export function arrowPath(points: readonly Point[]): string {
  if (points.length === 0) return '';
  let d = `M${points[0].x},${points[0].y}`;
  for (let i = 1; i < points.length; i++) {
    const p = points[i - 1];
    const q = points[i];
    const dx = (q.x - p.x) / 2;
    d += ` C${p.x + dx},${p.y} ${q.x - dx},${q.y} ${q.x},${q.y}`;
  }
  return d;
}

import { describe, expect, it } from 'vitest';
import { buildModel, hasStreamEvents } from './model';
import { layoutBoards, namespaceOf } from './multiboard';
import { type Body, curvePoint, distance, type Vec3 } from './ports';
import { storefrontEntities } from './scene3d.fixture';
import type { SceneNode, SceneSpec } from './scene3d';
import { buildTowers, FLOOR, SIDE_R, TOWER_GAP } from './towers';

const storefront = () => buildModel(storefrontEntities() as never);

function node(spec: SceneSpec, kind: string, slug: string): SceneNode {
  const n = spec.nodes.find((x) => x.entity.kind === kind && x.entity.id.slug === slug);
  if (!n) throw new Error(`no ${kind} ${slug}`);
  return n;
}

const towerX = (spec: SceneSpec, title: string) => spec.plates.find((p) => p.title === title)?.center.x ?? Number.NaN;

/** Distance outside the body's surface: negative inside, zero on it. */
function outside(p: Vec3, body: Body): number {
  const c = body.center;
  const s = body.shape;
  if (s.type === 'box') {
    return Math.max(
      Math.abs(p.x - c.x) - s.width / 2,
      Math.abs(p.y - c.y) - s.height / 2,
      Math.abs(p.z - c.z) - s.depth / 2,
    );
  }
  if (s.type === 'cylinder') {
    return Math.max(Math.hypot(p.x - c.x, p.z - c.z) - s.radius, Math.abs(p.y - c.y) - s.height / 2);
  }
  return distance(p, c) - s.radius;
}

describe('buildTowers', () => {
  it('raises one tower per stream lane, in swimlane order', () => {
    const spec = buildTowers(storefront());
    expect(spec.plates.map((p) => p.title)).toEqual(['Cart', 'Order', 'Fulfillment']);
    expect(spec.plates.map((p) => p.center.x)).toEqual([0, TOWER_GAP, 2 * TOWER_GAP]);
    expect(spec.plates[0].sub).toBe('cart-{cart_id}');
    expect(node(spec, 'event', 'cart.item-added').body.center.x).toBe(towerX(spec, 'Cart'));
    expect(node(spec, 'event', 'order.placed').body.center.x).toBe(towerX(spec, 'Order'));
    expect(node(spec, 'event', 'order.dispatched').body.center.x).toBe(towerX(spec, 'Fulfillment'));
  });

  it('stacks the events of a stream from the bottom up in Board column order', () => {
    const m = storefront();
    const spec = buildTowers(m);
    const boardX = new Map(layoutBoards(m).nodes.map((n) => [n.id, n.position.x]));
    const events = spec.nodes.filter((n) => n.entity.kind === 'event' && n.body.center.x === towerX(spec, 'Order'));
    expect(events.map((e) => e.entity.id.slug)).toEqual(expect.arrayContaining(['order.placed', 'payment.captured']));
    const byBoard = [...events].sort((a, b) => (boardX.get(a.id) ?? 0) - (boardX.get(b.id) ?? 0));
    const byHeight = [...events].sort((a, b) => a.body.center.y - b.body.center.y);
    expect(byHeight.map((e) => e.id)).toEqual(byBoard.map((e) => e.id));
  });

  it('does not share floor heights across bounded contexts that left-align to the same column', () => {
    const id = (namespace: string, slug: string) => ({ namespace, slug, version: '1' });
    const ref = (namespace: string, slug: string) => ({ id: id(namespace, slug) });
    const m = buildModel([
      { swimlane: { id: id('alpha', 'stream'), title: 'Alpha stream', order: 1 } },
      { event: { id: id('alpha', 'alpha.done'), title: 'Alpha done', swimlane: ref('alpha', 'stream') } },
      { swimlane: { id: id('beta', 'stream'), title: 'Beta stream', order: 1 } },
      { event: { id: id('beta', 'beta.done'), title: 'Beta done', swimlane: ref('beta', 'stream') } },
    ] as never);
    const spec = buildTowers(m);
    const alpha = spec.nodes.find((n) => n.entity.id.slug === 'alpha.done');
    const beta = spec.nodes.find((n) => n.entity.id.slug === 'beta.done');
    if (!alpha || !beta) throw new Error('expected both events');
    expect(alpha.body.center.y).not.toBe(beta.body.center.y);
  });

  it('lines floors up across towers: one Board column is one height', () => {
    const m = storefront();
    const spec = buildTowers(m);
    const boardX = new Map(layoutBoards(m).nodes.map((n) => [n.id, n.position.x]));
    for (const a of spec.nodes) {
      for (const b of spec.nodes) {
        const sameColumn = boardX.get(a.id) === boardX.get(b.id) && namespaceOf(a.id) === namespaceOf(b.id);
        expect(a.body.center.y === b.body.center.y).toBe(sameColumn);
      }
    }
  });

  it('attaches a command to the stream it appends to, on the floor of its event', () => {
    const spec = buildTowers(storefront());
    const command = node(spec, 'command', 'place-order').body.center;
    const event = node(spec, 'event', 'order.placed').body.center;
    expect(command.x).toBeCloseTo(towerX(spec, 'Order') + Math.cos((150 * Math.PI) / 180) * SIDE_R);
    expect(command.z).toBeCloseTo(Math.sin((150 * Math.PI) / 180) * SIDE_R);
    expect(command.y).toBe(event.y);
  });

  it('sets a read model beside the stream it projects from, linked from its events', () => {
    const spec = buildTowers(storefront());
    const rm = node(spec, 'readModel', 'pending-payments');
    const event = node(spec, 'event', 'order.placed');
    expect(rm.body.center.x).toBeCloseTo(towerX(spec, 'Order') + Math.cos(Math.PI / 6) * SIDE_R);
    expect(rm.body.center.y).toBeGreaterThan(event.body.center.y);
    expect(spec.links.some((l) => l.from === event.id && l.to === rm.id)).toBe(true);
  });

  it('stands an automation in front of the stream its command appends to', () => {
    const spec = buildTowers(storefront());
    const worker = node(spec, 'processor', 'payment-capture-worker').body.center;
    expect(worker.x).toBeCloseTo(towerX(spec, 'Order'));
    expect(worker.z).toBeCloseTo(SIDE_R);
  });

  it('draws events as thin discs and the rest as printed cards', () => {
    const spec = buildTowers(storefront());
    for (const n of spec.nodes) {
      if (n.entity.kind === 'event') {
        expect(n.body.shape.type).toBe('cylinder');
        expect(n.printed).toBe(false);
        expect(spec.tags.some((t) => t.related?.includes(n.entity.key) && t.tone === 'name')).toBe(true);
      } else {
        expect(n.body.shape.type).toBe('box');
        expect(n.printed).toBe(true);
      }
    }
  });

  it('leaves out screens and personas, which belong to no stream', () => {
    const kinds = new Set(buildTowers(storefront()).nodes.map((n) => n.entity.kind));
    expect([...kinds].sort()).toEqual(['command', 'event', 'processor', 'readModel']);
  });

  it('keeps the Board ids, so a selection and an edge id carry across views', () => {
    const m = storefront();
    const spec = buildTowers(m);
    const { nodes, edges } = layoutBoards(m);
    const stickyIds = new Set(nodes.filter((n) => n.type === 'sticky').map((n) => n.id));
    for (const n of spec.nodes) {
      expect(stickyIds.has(n.id)).toBe(true);
      expect(n.occId).toBe(n.id);
    }
    const edgeIds = new Set(edges.map((e) => e.id));
    expect(spec.links.length).toBeGreaterThan(0);
    for (const l of spec.links) expect(edgeIds.has(l.id)).toBe(true);
  });

  it('ends every link on the surface of its nodes, never at a center', () => {
    const spec = buildTowers(storefront());
    const bodies = new Map(spec.nodes.map((n) => [n.id, n.body]));
    for (const l of spec.links) {
      const from = bodies.get(l.from) as Body;
      const to = bodies.get(l.to) as Body;
      const start = l.points[0];
      const end = l.points[l.points.length - 1];
      expect(Math.abs(outside(start, from))).toBeLessThan(1e-6);
      expect(Math.abs(outside(end, to))).toBeLessThan(1e-6);
      expect(distance(start, from.center)).toBeGreaterThan(1);
      expect(distance(end, to.center)).toBeGreaterThan(1);
    }
  });

  it('routes links outside the nodes between their ends', () => {
    const spec = buildTowers(storefront());
    const crossings: string[] = [];
    for (const l of spec.links) {
      for (let t = 0.1; t <= 0.9; t += 0.05) {
        const p = curvePoint(l.points, t);
        for (const n of spec.nodes)
          if (outside(p, n.body) < -1e-6) crossings.push(`${l.id} at ${t.toFixed(2)} in ${n.id}`);
      }
    }
    expect(crossings).toEqual([]);
  });

  it('gives each event of a slice its own floor, in the order the Board lays them out', () => {
    const id = (slug: string) => ({ namespace: 'shop', slug, version: '1' });
    const ref = (slug: string) => ({ id: id(slug) });
    const m = buildModel([
      { swimlane: { id: id('order'), title: 'Order', order: 1 } },
      { command: { id: id('place'), title: 'Place' } },
      { event: { id: id('placed'), title: 'Placed', swimlane: ref('order') } },
      { event: { id: id('priced'), title: 'Priced', swimlane: ref('order') } },
      { readModel: { id: id('summary'), title: 'Summary', sourceEvents: [ref('placed')] } },
      {
        commandSlice: {
          id: id('place-slice'),
          title: 'Place',
          command: { command: ref('place') },
          emittedEvents: [{ event: ref('placed') }, { event: ref('priced') }],
        },
      },
      {
        readModelSlice: {
          id: id('summary-slice'),
          title: 'Summary',
          readModel: { readModel: ref('summary') },
          sourceEvents: [{ event: ref('placed') }],
        },
      },
      { storyboard: { id: id('flow'), title: 'Flow', slices: [ref('place-slice'), ref('summary-slice')] } },
    ] as never);
    const spec = buildTowers(m);
    const placed = node(spec, 'event', 'placed').body.center.y;
    const priced = node(spec, 'event', 'priced').body.center.y;
    expect(priced - placed).toBe(FLOOR);
    expect(node(spec, 'command', 'place').body.center.y).toBe(placed);
    expect(node(spec, 'readModel', 'summary').body.center.y).toBe(priced + FLOOR);
  });

  it('puts events without a swimlane on a tower of their own, as the Board does', () => {
    const m = buildModel([
      ...storefrontEntities(),
      { event: { id: { namespace: 'acme-storefront', slug: 'stray', version: '1' }, title: 'Stray' } },
    ] as never);
    const spec = buildTowers(m);
    const stray = node(spec, 'event', 'stray');
    const plate = spec.plates.find((p) => p.center.x === stray.body.center.x);
    expect(plate?.kicker).toBe('No swimlane');
    expect(plate?.entity).toBeUndefined();
  });

  it('gives a stream lane with no events no tower', () => {
    const m = buildModel([
      ...storefrontEntities(),
      { swimlane: { id: { namespace: 'acme-storefront', slug: 'returns', version: '1' }, title: 'Returns', order: 9 } },
    ] as never);
    expect(buildTowers(m).plates.map((p) => p.title)).toEqual(['Cart', 'Order', 'Fulfillment']);
  });

  it('steps through the slices that put something on a tower, in Board order', () => {
    const spec = buildTowers(storefront());
    const labels = spec.steps.map((s) => s.label);
    expect(labels.slice(0, 4)).toEqual(['Add item to cart', 'Cart summary', 'Place order', 'Pending payments']);
    // A screen slice only reads, so it has nothing on a tower.
    expect(labels).not.toContain('Shopper sees the order status');
    const placeOrder = spec.steps.find((s) => s.label === 'Place order');
    const y = node(spec, 'command', 'place-order').body.center.y;
    expect(placeOrder && y > placeOrder.region.min.y && y < placeOrder.region.max.y).toBe(true);
    expect(placeOrder?.highlight.facing).toBe('y');
  });

  it('draws an empty scene for a model without streams', () => {
    const spec = buildTowers(buildModel([]));
    expect(spec.nodes).toEqual([]);
    expect(spec.links).toEqual([]);
    expect(spec.steps).toEqual([]);
  });
});

describe('hasStreamEvents', () => {
  it('is true only when a swimlane holds an event', () => {
    expect(hasStreamEvents(storefront())).toBe(true);
    expect(hasStreamEvents(buildModel([]))).toBe(false);
    const id = (slug: string) => ({ namespace: 'shop', slug, version: '1' });
    const loose = buildModel([
      { swimlane: { id: id('order'), title: 'Order' } },
      { event: { id: id('placed'), title: 'Placed' } },
    ] as never);
    expect(hasStreamEvents(loose)).toBe(false);
  });
});

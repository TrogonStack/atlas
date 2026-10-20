import { describe, expect, it } from 'vitest';
import { CARD_DEPTH, buildLaneMesh, PX, WALL_BACK } from './lanemesh';
import { NODE_H, NODE_W, type SliceHeaderData } from './layout';
import { buildModel, isSeamEdge } from './model';
import { layoutBoards } from './multiboard';
import { type Body, curvePoint, distance, type Vec3 } from './ports';
import { storefrontEntities } from './scene3d.fixture';

const model = (codeRefs = false) => buildModel(storefrontEntities({ codeRefs }) as never);

function boxOf(body: Body) {
  if (body.shape.type !== 'box') throw new Error(`expected a box, got ${body.shape.type}`);
  return { c: body.center, hw: body.shape.width / 2, hh: body.shape.height / 2, hd: body.shape.depth / 2 };
}

function inside(p: Vec3, body: Body): boolean {
  const { c, hw, hh, hd } = boxOf(body);
  return Math.abs(p.x - c.x) < hw - 1e-6 && Math.abs(p.y - c.y) < hh - 1e-6 && Math.abs(p.z - c.z) < hd - 1e-6;
}

function onSurface(p: Vec3, body: Body): boolean {
  const { c, hw, hh, hd } = boxOf(body);
  const d = [Math.abs(p.x - c.x) / hw, Math.abs(p.y - c.y) / hh, Math.abs(p.z - c.z) / hd];
  return Math.max(...d) > 1 - 1e-6 && Math.max(...d) < 1 + 1e-6;
}

describe('buildLaneMesh', () => {
  it('puts every card at its Board position: x is the slice column and y is the lane row', () => {
    const m = model();
    const spec = buildLaneMesh(m);
    const board = layoutBoards(m).nodes.filter((n) => n.type === 'sticky');
    expect(spec.nodes).toHaveLength(board.length);
    for (const n of board) {
      const node = spec.nodes.find((s) => s.id === n.id);
      const w = Number(n.width ?? NODE_W);
      const h = Number(n.height ?? NODE_H);
      expect(node?.body.center.x).toBeCloseTo((n.position.x + w / 2) * PX);
      expect(node?.body.center.y).toBeCloseTo(-(n.position.y + h / 2) * PX);
      expect(node?.body.center.z).toBe(0);
      expect(node?.body.shape).toEqual({ type: 'box', width: w * PX, height: h * PX, depth: CARD_DEPTH });
    }
  });

  it('keeps the Board ids, so a selection and an edge id carry across views', () => {
    const m = model();
    const spec = buildLaneMesh(m);
    const { nodes, edges } = layoutBoards(m);
    for (const n of spec.nodes) {
      expect(n.occId).toBe(n.id);
      expect(nodes.some((b) => b.id === n.id)).toBe(true);
    }
    const edgeIds = new Set(edges.map((e) => e.id));
    for (const l of spec.links.filter((l) => l.kind === 'edge')) expect(edgeIds.has(l.id)).toBe(true);
  });

  it('carries the Board edge payload on every edge tube so a pick can select the connection', () => {
    const m = model();
    const spec = buildLaneMesh(m);
    const { edges } = layoutBoards(m);
    const byId = new Map(edges.map((e) => [e.id, e]));
    for (const l of spec.links.filter((l) => l.kind === 'edge')) {
      const board = byId.get(l.id);
      expect(l.edge).toEqual(board?.data);
      expect(l.edge?.source).toBeDefined();
      expect(l.edge?.target).toBeDefined();
    }
  });

  it('stacks the stream lanes in swimlane order, top to bottom', () => {
    const spec = buildLaneMesh(model());
    const rowOf = (slug: string) => {
      const event = spec.nodes.find((n) => n.entity.kind === 'event' && n.entity.id.slug === slug);
      return event?.body.center.y ?? Number.NaN;
    };
    expect(rowOf('cart.item-added')).toBeGreaterThan(rowOf('order.placed'));
    expect(rowOf('order.placed')).toBeGreaterThan(rowOf('order.dispatched'));
    const lanes = spec.plates.filter((p) => p.kicker === 'Event stream').map((p) => p.title);
    expect(lanes).toEqual(['Cart', 'Order', 'Fulfillment']);
  });

  it('labels a stream lane with its stream id', () => {
    const spec = buildLaneMesh(model());
    expect(spec.plates.find((p) => p.title === 'Cart')?.sub).toBe('cart-{cart_id}');
  });

  it('has one step per slice, in the Board order, each around its own cards', () => {
    const m = model();
    const spec = buildLaneMesh(m);
    const headers = layoutBoards(m).nodes.filter((n) => n.type === 'sliceHeader');
    expect(spec.steps.map((s) => s.label)).toEqual(headers.map((n) => (n.data as SliceHeaderData).entity.title));
    expect(spec.steps).toHaveLength(m.slices.length);
    const placeOrder = spec.steps.find((s) => s.label === 'Place order');
    const card = spec.nodes.find((n) => n.entity.id.slug === 'place-order' && n.entity.kind === 'command');
    const c = card?.body.center;
    expect(c && placeOrder && c.x > placeOrder.region.min.x && c.x < placeOrder.region.max.x).toBe(true);
    // Steps advance left to right, like the Board's columns.
    const xs = spec.steps.map((s) => (s.region.min.x + s.region.max.x) / 2);
    expect([...xs].sort((a, b) => a - b)).toEqual(xs);
  });

  it('ends every link on the surface of its cards, never at a card center', () => {
    const spec = buildLaneMesh(model(true));
    const bodies = new Map([...spec.nodes, ...spec.files].map((n) => [n.id, n.body]));
    expect(spec.links.length).toBeGreaterThan(0);
    for (const l of spec.links) {
      const from = bodies.get(l.from) as Body;
      const to = bodies.get(l.to) as Body;
      const start = l.points[0];
      const end = l.points[l.points.length - 1];
      expect(distance(start, from.center)).toBeGreaterThan(1);
      expect(distance(end, to.center)).toBeGreaterThan(1);
      expect(onSurface(start, from)).toBe(true);
      expect(onSurface(end, to)).toBe(true);
    }
  });

  it('routes links outside the cards between their ends', () => {
    const spec = buildLaneMesh(model(true));
    const crossings: string[] = [];
    for (const l of spec.links) {
      for (let t = 0.1; t <= 0.9; t += 0.05) {
        const p = curvePoint(l.points, t);
        for (const n of spec.nodes) if (inside(p, n.body)) crossings.push(`${l.id} at t=${t.toFixed(2)} in ${n.id}`);
      }
    }
    expect(crossings).toEqual([]);
  });

  it('leaves the board without a code wall when no entity cites code', () => {
    const spec = buildLaneMesh(model(false));
    expect(spec.files).toEqual([]);
    expect(spec.links.some((l) => l.kind === 'cite')).toBe(false);
    expect(spec.decor.some((d) => d.id === 'wall')).toBe(false);
    expect(spec.tags).toEqual([]);
    expect(spec.legend.some((line) => line.includes('code'))).toBe(false);
  });

  it('hangs the cited files under the board, bundled by repo and the first two directories', () => {
    const spec = buildLaneMesh(model(true));
    expect(spec.files.map((f) => f.path).sort()).toEqual([
      'apps/storefront/lib/storefront/cart/add_item.ex',
      'apps/storefront/lib/storefront/cart/events.ex',
      'apps/storefront/lib/storefront/orders/events.ex',
      'apps/storefront/lib/storefront/orders/place_order.ex',
      'apps/storefront_web/live/order_status_live.ex',
      'services/capture/events.go',
      'services/capture/worker.go',
    ]);
    const bundles = spec.tags.filter((t) => t.tone === 'zone').map((t) => t.text);
    expect(bundles.sort()).toEqual([
      'payments · services/capture',
      'storefront · apps/storefront',
      'storefront · apps/storefront_web',
    ]);
    const lowestCard = Math.min(...spec.nodes.map((n) => n.body.center.y - boxOf(n.body).hh));
    for (const f of spec.files) {
      expect(f.body.center.y).toBeLessThan(lowestCard);
      expect(f.body.center.z).toBe(-WALL_BACK);
    }
    expect(spec.legend.some((line) => line.includes('code'))).toBe(true);
  });

  it('draws one cite link per file from the first card of the citing entity, off its bottom rim', () => {
    const spec = buildLaneMesh(model(true));
    const cites = spec.links.filter((l) => l.kind === 'cite');
    const citing = spec.files.flatMap((f) => f.citedBy);
    expect(cites).toHaveLength(citing.length);
    for (const l of cites) {
      const card = spec.nodes.find((n) => n.id === l.from);
      if (!card) throw new Error(`no card ${l.from}`);
      const first = Math.min(...spec.nodes.filter((n) => n.entity.key === card.entity.key).map((n) => n.body.center.x));
      expect(card.body.center.x).toBe(first);
      const { c, hh } = boxOf(card.body);
      expect(l.points[0].y).toBeCloseTo(c.y - hh);
    }
  });

  it('frames the code wall in the Front and Overview presets', () => {
    const spec = buildLaneMesh(model(true));
    const lowestFile = Math.min(...spec.files.map((f) => f.body.center.y));
    expect(spec.front.min.y).toBeLessThan(lowestFile);
    expect(spec.overview.region.min.y).toBeLessThan(lowestFile);
  });

  it('draws an empty scene for an empty model', () => {
    const spec = buildLaneMesh(buildModel([]));
    expect(spec.nodes).toEqual([]);
    expect(spec.links).toEqual([]);
    expect(spec.steps).toEqual([]);
  });

  describe('with several contexts on one Board', () => {
    const cid = (namespace: string, slug: string) => ({ namespace, slug, version: '1' });
    const multi = () =>
      buildModel([
        { event: { id: cid('upstream', 'thing.published'), title: 'Thing published' } },
        {
          readModel: {
            id: cid('upstream', 'things'),
            title: 'Things',
            sourceEvents: [{ id: cid('upstream', 'thing.published') }],
          },
        },
        {
          readModel: {
            id: cid('downstream', 'thing-feed'),
            title: 'Thing feed',
            sourceEvents: [{ id: cid('upstream', 'thing.published') }],
          },
        },
      ] as never);

    it('keeps each context lane under its own namespaced id', () => {
      const spec = buildLaneMesh(multi());
      const laneNs = new Set(spec.plates.filter((p) => p.id.startsWith('lane:')).map((p) => p.id.split('::')[0]));
      expect(laneNs).toEqual(new Set(['lane:upstream', 'lane:downstream']));
    });

    it('stacks the contexts without overlapping their cards', () => {
      const spec = buildLaneMesh(multi());
      const ys = (ns: string) => spec.nodes.filter((n) => n.id.startsWith(`${ns}::`)).map((n) => boxOf(n.body));
      const up = ys('upstream');
      const down = ys('downstream');
      expect(up.length).toBeGreaterThan(0);
      expect(down.length).toBeGreaterThan(0);
      const top = (b: ReturnType<typeof boxOf>[]) => Math.max(...b.map((x) => x.c.y + x.hh));
      const bottom = (b: ReturnType<typeof boxOf>[]) => Math.min(...b.map((x) => x.c.y - x.hh));
      const [first, second] = top(up) > top(down) ? [up, down] : [down, up];
      expect(bottom(first)).toBeGreaterThan(top(second));
    });

    it('draws the seam link thicker than a plain edge', () => {
      const spec = buildLaneMesh(multi());
      const seam = spec.links.find((l) => l.kind === 'edge' && isSeamEdge(l.id));
      const plain = spec.links.find((l) => l.kind === 'edge' && !isSeamEdge(l.id));
      expect(seam).toBeDefined();
      if (plain && seam) expect(seam.radius).toBeGreaterThan(plain.radius);
    });
  });
});

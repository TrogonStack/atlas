// Decision #24 as a regression test: TIME ONLY POINTS FORWARD binds every
// derived graph. The sitemap is where the rule has been broken twice
// (type-level collapsing minted backward edges), so the invariant is now
// mechanical: every edge must strictly increase the occurrence layer.
import { describe, expect, it } from 'vitest';
import { buildModel, contextSeams, type EntityId } from './model';
import { buildSitemap } from './sitemap';

const id = (namespace: string, slug: string): Record<string, unknown> => ({ namespace, slug, version: '1' });

function fixtureEntities(): Record<string, unknown>[] {
  const ns = 'shop';
  return [
    { persona: { id: id(ns, 'buyer'), title: 'Buyer' } },
    { ui: { id: id(ns, 'home'), title: 'Home' } },
    { ui: { id: id(ns, 'detail'), title: 'Detail' } },
    { ui: { id: id(ns, 'orphan'), title: 'Orphan' } },
    { command: { id: id(ns, 'open-item'), title: 'open-item' } },
    { command: { id: id(ns, 'buy'), title: 'buy' } },
    { event: { id: id(ns, 'item.opened'), title: 'item.opened' } },
    { event: { id: id(ns, 'item.bought'), title: 'item.bought' } },
    { readModel: { id: id(ns, 'item-view'), title: 'item-view' } },
    { readModel: { id: id(ns, 'home-feed'), title: 'home-feed' } },
    {
      commandSlice: {
        id: id(ns, 's1-open'),
        title: 'open item',
        persona: { persona: { id: id(ns, 'buyer') } },
        ui: { ui: { id: id(ns, 'home') } },
        command: { command: { id: id(ns, 'open-item') } },
        emittedEvents: [{ event: { id: id(ns, 'item.opened') } }],
      },
    },
    {
      readModelSlice: {
        id: id(ns, 's2-view'),
        title: 'item view projects',
        readModel: { readModel: { id: id(ns, 'item-view') } },
        sourceEvents: [{ event: { id: id(ns, 'item.opened') } }],
      },
    },
    {
      uiSlice: {
        id: id(ns, 's2-view-ui'),
        title: 'detail shows the item view',
        persona: { persona: { id: id(ns, 'buyer') } },
        sourceReadModels: [{ readModel: { id: id(ns, 'item-view') } }],
        ui: { ui: { id: id(ns, 'detail') } },
      },
    },
    {
      commandSlice: {
        id: id(ns, 's3-buy'),
        title: 'buy',
        persona: { persona: { id: id(ns, 'buyer') } },
        ui: { ui: { id: id(ns, 'detail') } },
        command: { command: { id: id(ns, 'buy') } },
        emittedEvents: [{ event: { id: id(ns, 'item.bought') } }],
      },
    },
    {
      readModelSlice: {
        id: id(ns, 's4-back-home'),
        title: 'home feed updates',
        readModel: { readModel: { id: id(ns, 'home-feed') } },
        sourceEvents: [{ event: { id: id(ns, 'item.bought') } }],
      },
    },
    {
      uiSlice: {
        id: id(ns, 's4-back-home-ui'),
        title: 'home shows the feed',
        persona: { persona: { id: id(ns, 'buyer') } },
        sourceReadModels: [{ readModel: { id: id(ns, 'home-feed') } }],
        ui: { ui: { id: id(ns, 'home') } },
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-buy-flow'),
        title: 'buy flow',
        slices: [
          { id: id(ns, 's1-open') },
          { id: id(ns, 's2-view') },
          { id: id(ns, 's2-view-ui') },
          { id: id(ns, 's3-buy') },
          { id: id(ns, 's4-back-home') },
          { id: id(ns, 's4-back-home-ui') },
        ],
      },
    },
  ];
}

function layerOf(nodes: { id: string; position: { x: number } }[], nodeId: string): number {
  const n = nodes.find((x) => x.id === nodeId);
  if (!n) throw new Error(`node ${nodeId} missing`);
  return n.position.x;
}

describe('sitemap: forward-only (Decision #24)', () => {
  it('never emits a backward or self edge, even when the journey returns home', () => {
    const model = buildModel(fixtureEntities());
    const dropped: string[] = [];
    const { nodes, edges } = buildSitemap(model, (from, to) => dropped.push(`${from}->${to}`));
    expect(edges.length).toBeGreaterThan(0);
    expect(dropped).toHaveLength(0);
    for (const e of edges) {
      expect(e.source).not.toBe(e.target);
      expect(layerOf(nodes as never, e.source)).toBeLessThan(layerOf(nodes as never, e.target));
    }
  });

  it('unrolls a revisited surface into a new occurrence with #n/m grouping', () => {
    const model = buildModel(fixtureEntities());
    const { nodes } = buildSitemap(model);
    const homes = nodes.filter((n) => (n.data as { ui?: { id: { slug: string } } }).ui?.id.slug === 'home');
    expect(homes.length).toBe(2);
    const instances = homes.map((n) => (n.data as { instance?: string }).instance).sort();
    expect(instances).toEqual(['#1/2', '#2/2']);
  });

  it('unrolls declared navigation cycles instead of pointing backward', () => {
    const entities = fixtureEntities();
    const detail = entities.find((e) => (e.ui as { id?: { slug?: string } } | undefined)?.id?.slug === 'detail');
    (detail as { ui: Record<string, unknown> }).ui.transitions = [
      { to: { id: id('shop', 'home') }, doc: 'back to home' },
    ];
    const model = buildModel(entities);
    const { nodes, edges } = buildSitemap(model);
    for (const e of edges) {
      expect(layerOf(nodes as never, e.source)).toBeLessThan(layerOf(nodes as never, e.target));
    }
    const declared = edges.find((e) => (e.data as { relation: string }).relation.startsWith('declared'));
    expect(declared).toBeDefined();
  });

  it('keeps untouched surfaces in the inventory as isolated nodes', () => {
    const model = buildModel(fixtureEntities());
    const { nodes } = buildSitemap(model);
    expect(nodes.some((n) => (n.data as { ui?: { id: { slug: string } } }).ui?.id.slug === 'orphan')).toBe(true);
  });

  // BUG: bySlug last-wins on entity order; layout/plan use versionOf max.
  it('resolves the latest UI version when multiple versions exist', () => {
    const entities = fixtureEntities();
    const homeIdx = entities.findIndex((e) => (e.ui as { id?: { slug?: string } } | undefined)?.id?.slug === 'home');
    entities.splice(homeIdx, 1, { ui: { id: { namespace: 'shop', slug: 'home', version: '2' }, title: 'Home v2' } });
    entities.push({ ui: { id: { namespace: 'shop', slug: 'home', version: '1' }, title: 'Home v1' } });
    const model = buildModel(entities);
    const { nodes } = buildSitemap(model);
    const homes = nodes.filter((n) => (n.data as { ui?: { id: { slug: string } } }).ui?.id.slug === 'home');
    expect(homes.length).toBeGreaterThan(0);
    for (const n of homes) {
      const ui = (n.data as { ui: { title: string; id: { version: string } } }).ui;
      expect(ui.title).toBe('Home v2');
      expect(ui.id.version).toBe('2');
    }
  });
});

describe('context seams', () => {
  it('derives exactly the event → subscription-view crossings', () => {
    const upstream = { event: { id: { namespace: 'up', slug: 'thing.done', version: '1' }, title: 'thing.done' } };
    const view = {
      readModel: {
        id: { namespace: 'down', slug: 'things.done', version: '1' },
        title: 'things.done',
        sourceEvents: [{ id: { namespace: 'up', slug: 'thing.done', version: '1' } }],
      },
    };
    const local = {
      readModel: {
        id: { namespace: 'down', slug: 'local.view', version: '1' },
        title: 'local.view',
        sourceEvents: [{ id: { namespace: 'down', slug: 'local.evt', version: '1' } }],
      },
    };
    const model = buildModel([upstream, view, local] as Record<string, unknown>[]);
    const seams = contextSeams(model);
    expect(seams.length).toBe(1);
    const seam = seams[0] as { upstreamEvent: EntityId; view: { id: EntityId } };
    expect(seam.upstreamEvent.namespace).toBe('up');
    expect(seam.view.id.namespace).toBe('down');
  });
});

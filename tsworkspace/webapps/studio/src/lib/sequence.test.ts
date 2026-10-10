// Decision #24 in the sequence view: time only points forward. The
// sequence layout places stickies on a strict left-to-right time axis;
// every edge in the derived graph must traverse forward in that axis.
// We pin the invariant here so a refactor cannot quietly reintroduce
// backward references through the slot machinery.
import type { Node } from '@xyflow/react';
import { describe, expect, it } from 'vitest';
import { buildModel } from './model';
import { buildSequence } from './sequence';

const id = (namespace: string, slug: string): Record<string, unknown> => ({
  namespace,
  slug,
  version: '1',
});

function fixtureEntities(): Record<string, unknown>[] {
  const ns = 'shop';
  return [
    { persona: { id: id(ns, 'buyer'), title: 'Buyer' } },
    { ui: { id: id(ns, 'home'), title: 'Home' } },
    { ui: { id: id(ns, 'detail'), title: 'Detail' } },
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

// A data-deletion pipeline shape that produced a "read model happens next
// read model" arrow: a run of projections, then an automation reading one of
// them, then the command slice handling the command it emits. Both the read
// model and the command used to appear twice in adjacent columns, so a card
// pointed at a copy of itself and its same-moment siblings pointed at it too.
function dedupeFixture(): Record<string, unknown>[] {
  const ns = 'deletion';
  return [
    { event: { id: id(ns, 'deletion.started'), title: 'Deletion started' } },
    { readModel: { id: id(ns, 'analytics-queue'), title: 'Marketing deletion pending' } },
    { readModel: { id: id(ns, 'backoffice-queue'), title: 'Back-office redaction pending' } },
    { processor: { id: id(ns, 'analytics-failure-recorder'), title: 'Marketing failure recorder' } },
    { command: { id: id(ns, 'fail-analytics'), title: 'Fail marketing deletion' } },
    { event: { id: id(ns, 'analytics.failed'), title: 'Marketing deletion failed' } },
    {
      readModelSlice: {
        id: id(ns, 'analytics-queue-rms'),
        title: 'marketing queue projects',
        readModel: { readModel: { id: id(ns, 'analytics-queue') } },
        sourceEvents: [{ event: { id: id(ns, 'deletion.started') } }],
      },
    },
    {
      readModelSlice: {
        id: id(ns, 'backoffice-queue-rms'),
        title: 'back-office queue projects',
        readModel: { readModel: { id: id(ns, 'backoffice-queue') } },
        sourceEvents: [{ event: { id: id(ns, 'deletion.started') } }],
      },
    },
    {
      automationSlice: {
        id: id(ns, 'analytics-failure-auto'),
        title: 'record marketing-deletion failure',
        processor: { processor: { id: id(ns, 'analytics-failure-recorder') } },
        emittedCommand: { command: { id: id(ns, 'fail-analytics') } },
        sourceReadModels: [{ readModel: { id: id(ns, 'analytics-queue') } }],
      },
    },
    {
      commandSlice: {
        id: id(ns, 'fail-analytics-slice'),
        title: 'fail marketing deletion',
        command: { command: { id: id(ns, 'fail-analytics') } },
        emittedEvents: [{ event: { id: id(ns, 'analytics.failed') } }],
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-personal-data'),
        title: 'personal data',
        slices: [
          { id: id(ns, 'analytics-queue-rms') },
          { id: id(ns, 'backoffice-queue-rms') },
          { id: id(ns, 'analytics-failure-auto') },
          { id: id(ns, 'fail-analytics-slice') },
        ],
      },
    },
  ];
}

function stickiesFor(nodes: Node[], slug: string): Node[] {
  return nodes.filter(
    (n) => n.type === 'seqSticky' && (n.data as { entity: { id: { slug: string } } }).entity.id.slug === slug,
  );
}

function stickyKinds(nodes: Node[]): Map<string, string> {
  return new Map(
    nodes
      .filter((n) => n.type === 'seqSticky')
      .map((n) => [n.id, (n.data as { entity: { kind: string } }).entity.kind] as const),
  );
}

describe('buildSequence: forward-only (Decision #24)', () => {
  it('produces a non-empty graph for the buy-flow storyboard', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes } = buildSequence(model);
    expect(nodes.length).toBeGreaterThan(0);
  });

  it('only emits edges whose endpoints exist in the node set', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes, edges } = buildSequence(model);
    const ids = new Set(nodes.map((n) => n.id));
    for (const e of edges) {
      expect(ids.has(e.source), `dangling source ${e.source} on edge ${e.id}`).toBe(true);
      expect(ids.has(e.target), `dangling target ${e.target} on edge ${e.id}`).toBe(true);
    }
  });

  it('places every causal edge with source.x strictly less than target.x', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes, edges } = buildSequence(model);
    const byId = new Map(nodes.map((n) => [n.id, n] as const));
    for (const e of edges) {
      const s = byId.get(e.source);
      const t = byId.get(e.target);
      if (!s || !t) continue;
      // Sticky bands (storyboards/slices) overlap their members in x;
      // skip those derived bands and only enforce the rule on stickies.
      if (s.type === 'seqSticky' && t.type === 'seqSticky') {
        expect(s.position.x, `backward edge ${e.source}→${e.target} (${s.position.x} → ${t.position.x})`).toBeLessThan(
          t.position.x,
        );
      }
    }
  });

  it('forward-only guard filters on the sticky type buildSequence actually emits', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes, edges } = buildSequence(model);
    const byId = new Map(nodes.map((n) => [n.id, n] as const));
    let stickyPairs = 0;
    for (const e of edges) {
      const s = byId.get(e.source);
      const t = byId.get(e.target);
      if (!s || !t) continue;
      if (s.type === 'seqSticky' && t.type === 'seqSticky') stickyPairs++;
    }
    expect(stickyPairs).toBeGreaterThan(0);
  });
});

describe('buildSequence: automation and same-moment packing', () => {
  it('includes source read models, processor, and emitted command for an automation slice', () => {
    const ns = 'shop';
    const model = buildModel([
      { readModel: { id: id(ns, 'rpc-requests'), title: 'RPC requests' } },
      { processor: { id: id(ns, 'intake'), title: 'Intake' } },
      { command: { id: id(ns, 'initiate'), title: 'Initiate' } },
      {
        automationSlice: {
          id: id(ns, 's1-intake'),
          title: 'intake',
          processor: { processor: { id: id(ns, 'intake') } },
          emittedCommand: { command: { id: id(ns, 'initiate') } },
          sourceReadModels: [{ readModel: { id: id(ns, 'rpc-requests') } }],
        },
      },
      {
        storyboard: {
          id: id(ns, 'sb-intake'),
          title: 'intake flow',
          slices: [{ id: id(ns, 's1-intake') }],
        },
      },
    ] as never);
    const { nodes } = buildSequence(model);
    const slugs = nodes
      .filter((n) => n.type === 'seqSticky')
      .map((n) => (n.data as { entity: { id: { slug: string } } }).entity.id.slug);
    expect(slugs).toContain('rpc-requests');
    expect(slugs).toContain('intake');
    expect(slugs).toContain('initiate');
  });

  it('places a UI slice screen in the column after the read model it displays', () => {
    const ns = 'shop';
    const model = buildModel([
      { ui: { id: id(ns, 'dashboard'), title: 'Dashboard' } },
      { event: { id: id(ns, 'order.placed'), title: 'Order placed' } },
      { readModel: { id: id(ns, 'orders-feed'), title: 'Orders feed' } },
      {
        readModelSlice: {
          id: id(ns, 's1-feed'),
          title: 'feed projects',
          readModel: { readModel: { id: id(ns, 'orders-feed') } },
          sourceEvents: [{ event: { id: id(ns, 'order.placed') } }],
        },
      },
      {
        uiSlice: {
          id: id(ns, 's2-dashboard'),
          title: 'dashboard shows the feed',
          sourceReadModels: [{ readModel: { id: id(ns, 'orders-feed') } }],
          ui: { ui: { id: id(ns, 'dashboard') } },
        },
      },
      {
        storyboard: {
          id: id(ns, 'sb-feed'),
          title: 'feed',
          slices: [{ id: id(ns, 's1-feed') }, { id: id(ns, 's2-dashboard') }],
        },
      },
    ] as never);
    const { nodes } = buildSequence(model);
    const stickies = nodes.filter((n) => n.type === 'seqSticky');
    const slugs = stickies.map((n) => (n.data as { entity: { id: { slug: string } } }).entity.id.slug);
    expect(slugs).toContain('orders-feed');
    expect(slugs).toContain('dashboard');
    const rm = stickies.find((n) => (n.data as { entity: { id: { slug: string } } }).entity.id.slug === 'orders-feed');
    const ui = stickies.find((n) => (n.data as { entity: { id: { slug: string } } }).entity.id.slug === 'dashboard');
    expect(rm).toBeDefined();
    expect(ui).toBeDefined();
    // Its own column to the right, on the same row: a screen sits on the
    // time axis after the read model it displays, never stacked under it.
    expect(ui?.position.x).toBeGreaterThan(rm?.position.x ?? 0);
    expect(ui?.position.y).toBe(rm?.position.y);
  });

  it('enters a UI slice screen from its read model, never straight from the event', () => {
    const { nodes, edges } = buildSequence(buildModel(fixtureEntities() as never));
    const kindOf = stickyKinds(nodes);
    const pairs = edges
      .map((e) => [kindOf.get(e.source), kindOf.get(e.target)] as const)
      .filter(([s, t]) => s !== undefined && t !== undefined)
      .map(([s, t]) => `${s}->${t}`);
    expect(pairs).not.toContain('event->ui');
    expect(pairs).toContain('event->readModel');
    expect(pairs).toContain('readModel->ui');
  });

  it('does not re-draw a read model the projection to its left already placed', () => {
    const ns = 'shop';
    const model = buildModel([
      { ui: { id: id(ns, 'detail'), title: 'Detail' } },
      { event: { id: id(ns, 'item.opened'), title: 'item.opened' } },
      { readModel: { id: id(ns, 'item-view'), title: 'item-view' } },
      {
        readModelSlice: {
          id: id(ns, 's1-view'),
          title: 'item view projects',
          readModel: { readModel: { id: id(ns, 'item-view') } },
          sourceEvents: [{ event: { id: id(ns, 'item.opened') } }],
        },
      },
      {
        uiSlice: {
          id: id(ns, 's2-detail'),
          title: 'detail shows the item view',
          sourceReadModels: [{ readModel: { id: id(ns, 'item-view') } }],
          ui: { ui: { id: id(ns, 'detail') } },
        },
      },
      {
        storyboard: {
          id: id(ns, 'sb-open'),
          title: 'open',
          slices: [{ id: id(ns, 's1-view') }, { id: id(ns, 's2-detail') }],
        },
      },
    ] as never);
    const { nodes, edges } = buildSequence(model);
    expect(stickiesFor(nodes, 'item-view')).toHaveLength(1);
    const [detail] = stickiesFor(nodes, 'detail');
    const kindOf = stickyKinds(nodes);
    expect(edges.filter((e) => e.target === detail.id).map((e) => kindOf.get(e.source))).toEqual(['readModel']);
  });

  it('bands each stacked projection over its own card', () => {
    const ns = 'shop';
    const model = buildModel([
      { event: { id: id(ns, 'opened'), title: 'opened' } },
      { readModel: { id: id(ns, 'rm-a'), title: 'RM A' } },
      { readModel: { id: id(ns, 'rm-b'), title: 'RM B' } },
      { ui: { id: id(ns, 'screen-b'), title: 'Screen B' } },
      {
        readModelSlice: {
          id: id(ns, 'r1'),
          title: 'a projects',
          readModel: { readModel: { id: id(ns, 'rm-a') } },
          sourceEvents: [{ event: { id: id(ns, 'opened') } }],
        },
      },
      {
        readModelSlice: {
          id: id(ns, 'r2'),
          title: 'b projects',
          readModel: { readModel: { id: id(ns, 'rm-b') } },
          sourceEvents: [{ event: { id: id(ns, 'opened') } }],
        },
      },
      {
        uiSlice: {
          id: id(ns, 'u1'),
          title: 'screen b shows rm-b',
          sourceReadModels: [{ readModel: { id: id(ns, 'rm-b') } }],
          ui: { ui: { id: id(ns, 'screen-b') } },
        },
      },
      {
        storyboard: {
          id: id(ns, 'sb'),
          title: 'sb',
          slices: [{ id: id(ns, 'r1') }, { id: id(ns, 'r2') }, { id: id(ns, 'u1') }],
        },
      },
    ] as never);
    const { nodes } = buildSequence(model);
    const [rmA] = stickiesFor(nodes, 'rm-a');
    const [rmB] = stickiesFor(nodes, 'rm-b');
    const [screen] = stickiesFor(nodes, 'screen-b');
    // rm-b stacks under rm-a in the same column, so its band hangs over its
    // own card instead of sharing the header row at the top of the chunk.
    expect(rmB.position.x).toBe(rmA.position.x);
    expect(rmB.position.y).toBeGreaterThan(rmA.position.y);
    const bandFor = (slug: string) =>
      nodes.filter(
        (n) => n.type === 'seqSliceHeader' && (n.data as { entity: { id: { slug: string } } }).entity.id.slug === slug,
      );
    for (const [slug, card] of [
      ['r1', rmA],
      ['r2', rmB],
      ['u1', screen],
    ] as const) {
      const [band] = bandFor(slug);
      expect(band, `no band for ${slug}`).toBeDefined();
      expect(band.position.x).toBe(card.position.x);
      expect(band.position.y).toBeLessThan(card.position.y);
    }
    expect(new Set(nodes.map((n) => n.id)).size).toBe(nodes.length);
  });

  it('reads through a screen column: an automation still enters from the read model', () => {
    const ns = 'shop';
    const model = buildModel([
      { event: { id: id(ns, 'started'), title: 'Started' } },
      { readModel: { id: id(ns, 'queue'), title: 'Queue' } },
      { ui: { id: id(ns, 'queue-screen'), title: 'Queue screen' } },
      { processor: { id: id(ns, 'worker'), title: 'Worker' } },
      { command: { id: id(ns, 'do-it'), title: 'Do it' } },
      {
        readModelSlice: {
          id: id(ns, 'r1'),
          title: 'queue projects',
          readModel: { readModel: { id: id(ns, 'queue') } },
          sourceEvents: [{ event: { id: id(ns, 'started') } }],
        },
      },
      {
        uiSlice: {
          id: id(ns, 'u1'),
          title: 'queue screen shows the queue',
          sourceReadModels: [{ readModel: { id: id(ns, 'queue') } }],
          ui: { ui: { id: id(ns, 'queue-screen') } },
        },
      },
      {
        automationSlice: {
          id: id(ns, 'a1'),
          title: 'work it',
          processor: { processor: { id: id(ns, 'worker') } },
          emittedCommand: { command: { id: id(ns, 'do-it') } },
          sourceReadModels: [{ readModel: { id: id(ns, 'queue') } }],
        },
      },
      {
        storyboard: {
          id: id(ns, 'sb'),
          title: 'sb',
          slices: [{ id: id(ns, 'r1') }, { id: id(ns, 'u1') }, { id: id(ns, 'a1') }],
        },
      },
    ] as never);
    const { nodes, edges } = buildSequence(model);
    // The screen takes a column of its own, so the read model it displays is
    // no longer the processor's immediate neighbour. The chain must read
    // through that column rather than around it.
    expect(stickiesFor(nodes, 'queue')).toHaveLength(1);
    const [processor] = stickiesFor(nodes, 'worker');
    const kindOf = stickyKinds(nodes);
    const into = edges.filter((e) => e.target === processor.id).map((e) => kindOf.get(e.source));
    expect(into).toEqual(['readModel']);
  });

  it('reads sibling projections as siblings: no read model points at a read model', () => {
    const { nodes, edges } = buildSequence(buildModel(dedupeFixture() as never));
    const kindOf = stickyKinds(nodes);
    const sameKind = edges.filter((e) => {
      const s = kindOf.get(e.source);
      const t = kindOf.get(e.target);
      return s !== undefined && s === t;
    });
    expect(sameKind.map((e) => `${e.source}→${e.target}`)).toEqual([]);
  });

  it('does not re-draw a source read model the previous moment already projects', () => {
    const { nodes } = buildSequence(buildModel(dedupeFixture() as never));
    expect(stickiesFor(nodes, 'analytics-queue')).toHaveLength(1);
  });

  it('enters a processor only from the state it observes, not from same-moment siblings', () => {
    const { nodes, edges } = buildSequence(buildModel(dedupeFixture() as never));
    const [processor] = stickiesFor(nodes, 'analytics-failure-recorder');
    const slugOf = new Map(
      nodes
        .filter((n) => n.type === 'seqSticky')
        .map((n) => [n.id, (n.data as { entity: { id: { slug: string } } }).entity.id.slug] as const),
    );
    const inbound = edges.filter((e) => e.target === processor.id).map((e) => slugOf.get(e.source));
    expect(inbound).toEqual(['analytics-queue']);
  });

  it('does not re-draw a command the preceding automation already emitted', () => {
    const { nodes, edges } = buildSequence(buildModel(dedupeFixture() as never));
    const commands = stickiesFor(nodes, 'fail-analytics');
    expect(commands).toHaveLength(1);
    const [event] = stickiesFor(nodes, 'analytics.failed');
    const inbound = edges.filter((e) => e.target === event.id).map((e) => e.source);
    expect(inbound).toEqual([commands[0].id]);
  });

  it('stacks an automation’s source read models in one moment instead of chaining them', () => {
    const ns = 'shop';
    const { nodes } = buildSequence(
      buildModel([
        { readModel: { id: id(ns, 'quota'), title: 'Quota' } },
        { readModel: { id: id(ns, 'backlog'), title: 'Backlog' } },
        { processor: { id: id(ns, 'scheduler'), title: 'Scheduler' } },
        { command: { id: id(ns, 'schedule'), title: 'Schedule' } },
        {
          automationSlice: {
            id: id(ns, 's1-schedule'),
            title: 'schedule',
            processor: { processor: { id: id(ns, 'scheduler') } },
            emittedCommand: { command: { id: id(ns, 'schedule') } },
            sourceReadModels: [{ readModel: { id: id(ns, 'quota') } }, { readModel: { id: id(ns, 'backlog') } }],
          },
        },
        { storyboard: { id: id(ns, 'sb'), title: 'sb', slices: [{ id: id(ns, 's1-schedule') }] } },
      ] as never),
    );
    const [quota] = stickiesFor(nodes, 'quota');
    const [backlog] = stickiesFor(nodes, 'backlog');
    expect(Math.round(quota.position.x)).toBe(Math.round(backlog.position.x));
  });

  it('fans co-emitted events from the command instead of chaining them as successive moments', () => {
    const ns = 'shop';
    const model = buildModel([
      { ui: { id: id(ns, 'checkout'), title: 'Checkout' } },
      { command: { id: id(ns, 'place-order'), title: 'Place order' } },
      { event: { id: id(ns, 'order.placed'), title: 'Order placed' } },
      { event: { id: id(ns, 'payment.requested'), title: 'Payment requested' } },
      {
        commandSlice: {
          id: id(ns, 's1-place-order'),
          title: 'place order',
          ui: { ui: { id: id(ns, 'checkout') } },
          command: { command: { id: id(ns, 'place-order') } },
          emittedEvents: [{ event: { id: id(ns, 'order.placed') } }, { event: { id: id(ns, 'payment.requested') } }],
        },
      },
      {
        storyboard: {
          id: id(ns, 'sb-place-order'),
          title: 'place order',
          slices: [{ id: id(ns, 's1-place-order') }],
        },
      },
    ] as never);
    const { nodes, edges } = buildSequence(model);
    const bySlug = new Map(
      nodes
        .filter((n) => n.type === 'seqSticky')
        .map((n) => [(n.data as { entity: { id: { slug: string } } }).entity.id.slug, n.id] as const),
    );
    const cmd = bySlug.get('place-order');
    const e1 = bySlug.get('order.placed');
    const e2 = bySlug.get('payment.requested');
    expect(cmd && e1 && e2).toBeTruthy();
    const fromCommand = edges
      .filter((e) => e.source === cmd)
      .map((e) => e.target)
      .sort();
    expect(fromCommand).toEqual([e1, e2].sort());
    const betweenEvents = edges.filter(
      (e) => (e.source === e1 && e.target === e2) || (e.source === e2 && e.target === e1),
    );
    expect(betweenEvents).toHaveLength(0);
  });
});

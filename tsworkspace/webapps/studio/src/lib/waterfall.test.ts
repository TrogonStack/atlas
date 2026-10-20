// Tests for buildWaterfall in waterfall.ts.
// Covers: chain ownership, join and above rows, the group of entities no
// storyboard reaches, lane x kind groups, cable counts, and code bundles.

import { describe, expect, it } from 'vitest';
import { buildModel, type Model } from './model';
import {
  buildWaterfall,
  codeBundles,
  happensNextLinks,
  NOT_IN_A_STORYBOARD,
  type Waterfall,
  type WaterfallGroup,
} from './waterfall';

const NS = 'acme-storefront';
const id = (slug: string, namespace = NS, version = '1') => ({ namespace, slug, version });
const ref = (slug: string, namespace = NS) => ({ id: id(slug, namespace) });

const codeRef = (repo: string, path: string) => ({
  '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.CodeRefAnnotation',
  repo,
  path,
  symbol: 'Handle',
});

// A small storefront: checkout places an order, a payment chain captures
// it, and a refund nobody put in a storyboard yet.
function storefront(opts: { code?: boolean } = {}): Record<string, unknown>[] {
  const code = opts.code ?? false;
  return [
    { persona: { id: id('shopper'), title: 'Shopper' } },
    { swimlane: { id: id('cart'), title: 'Cart', order: 1 } },
    { swimlane: { id: id('order'), title: 'Order', order: 2 } },
    { ui: { id: id('storefront-checkout'), title: 'Checkout page' } },
    { ui: { id: id('order-status-page'), title: 'Order status page' } },
    { command: { id: id('add-item-to-cart'), title: 'Add item to cart' } },
    {
      command: {
        id: id('place-order'),
        title: 'Place order',
        metadata: code
          ? [
              codeRef('storefront', 'services/checkout/place_order.go'),
              codeRef('storefront', 'services/checkout/handlers/http.go'),
            ]
          : [],
      },
    },
    { command: { id: id('capture-payment'), title: 'Capture payment' } },
    { command: { id: id('issue-refund'), title: 'Issue refund' } },
    { event: { id: id('cart.item-added'), title: 'Item added', swimlane: ref('cart') } },
    { event: { id: id('order.placed'), title: 'Order placed', swimlane: ref('order') } },
    { event: { id: id('payment.captured'), title: 'Payment captured', swimlane: ref('order') } },
    { event: { id: id('refund.issued'), title: 'Refund issued', swimlane: ref('order') } },
    { event: { id: id('order.archived'), title: 'Order archived', swimlane: ref('order') } },
    { readModel: { id: id('cart-summary'), title: 'Cart summary' } },
    { readModel: { id: id('pending-payments'), title: 'Pending payments' } },
    {
      readModel: {
        id: id('order-status'),
        title: 'Order status',
        metadata: code ? [codeRef('storefront', 'services/orders/status/projection.go')] : [],
      },
    },
    {
      processor: {
        id: id('payment-capturer'),
        title: 'Payment capturer',
        metadata: code ? [codeRef('payments', 'workers/capture.go'), codeRef('payments', 'main.go')] : [],
      },
    },
    {
      commandSlice: {
        id: id('add-item-to-cart-slice'),
        title: 'Add item to cart',
        persona: { persona: ref('shopper') },
        ui: { ui: ref('storefront-checkout') },
        command: { command: ref('add-item-to-cart') },
        emittedEvents: [{ event: ref('cart.item-added') }],
      },
    },
    {
      readModelSlice: {
        id: id('cart-summary-rms'),
        title: 'Cart summary',
        readModel: { readModel: ref('cart-summary') },
        sourceEvents: [{ event: ref('cart.item-added') }],
      },
    },
    {
      commandSlice: {
        id: id('place-order-slice'),
        title: 'Place order',
        persona: { persona: ref('shopper') },
        ui: { ui: ref('storefront-checkout') },
        command: { command: ref('place-order') },
        emittedEvents: [{ event: ref('order.placed') }],
      },
    },
    {
      readModelSlice: {
        id: id('pending-payments-rms'),
        title: 'Pending payments',
        readModel: { readModel: ref('pending-payments') },
        sourceEvents: [{ event: ref('order.placed') }],
      },
    },
    {
      readModelSlice: {
        id: id('order-status-placed-rms'),
        title: 'Order status, placed',
        readModel: { readModel: ref('order-status') },
        sourceEvents: [{ event: ref('order.placed') }],
      },
    },
    {
      automationSlice: {
        id: id('capture-payment-auto'),
        title: 'Capture payment',
        processor: { processor: ref('payment-capturer') },
        emittedCommand: { command: ref('capture-payment') },
        sourceReadModels: [{ readModel: ref('pending-payments') }],
      },
    },
    {
      commandSlice: {
        id: id('capture-payment-slice'),
        title: 'Capture payment',
        command: { command: ref('capture-payment') },
        emittedEvents: [{ event: ref('payment.captured') }],
      },
    },
    {
      readModelSlice: {
        id: id('order-status-paid-rms'),
        title: 'Order status, paid',
        readModel: { readModel: ref('order-status') },
        sourceEvents: [{ event: ref('payment.captured') }],
      },
    },
    {
      uiSlice: {
        id: id('order-status-ui'),
        title: 'Shopper sees order status',
        persona: { persona: ref('shopper') },
        ui: { ui: ref('order-status-page') },
        sourceReadModels: [{ readModel: ref('order-status') }],
      },
    },
    {
      commandSlice: {
        id: id('issue-refund-slice'),
        title: 'Issue refund',
        command: { command: ref('issue-refund') },
        emittedEvents: [{ event: ref('refund.issued') }],
      },
    },
    {
      readModelSlice: {
        id: id('order-status-refunded-rms'),
        title: 'Order status, refunded',
        readModel: { readModel: ref('order-status') },
        sourceEvents: [{ event: ref('refund.issued') }],
      },
    },
    {
      storyboard: {
        id: id('checkout-flow'),
        title: 'Checkout',
        slices: [
          ref('add-item-to-cart-slice'),
          ref('cart-summary-rms'),
          ref('place-order-slice'),
          ref('pending-payments-rms'),
          ref('order-status-placed-rms'),
        ],
      },
    },
    {
      storyboard: {
        id: id('payment-flow'),
        title: 'Payment',
        slices: [
          ref('capture-payment-auto'),
          ref('capture-payment-slice'),
          ref('order-status-paid-rms'),
          ref('order-status-ui'),
        ],
      },
    },
  ];
}

const build = (entities: Record<string, unknown>[]): { model: Model; wf: Waterfall } => {
  const model = buildModel(entities as never);
  return { model, wf: buildWaterfall(model) };
};

function group(groups: WaterfallGroup[], title: string, lane?: string): WaterfallGroup {
  const g = groups.find((x) => x.title === title && (lane === undefined || x.lane === lane));
  if (!g) throw new Error(`no group ${title} ${lane ?? ''}; have ${groups.map((x) => `${x.title}/${x.lane}`)}`);
  return g;
}

const describeRows = (g: WaterfallGroup) => g.rows.map((r) => `${r.type}:${r.entity.id.slug}`);

describe('happensNextLinks', () => {
  it('derives the Board relations once per entity pair', () => {
    const { model } = build(storefront());
    const links = happensNextLinks(model).map((l) => `${l.from.id.slug} ${l.relation} ${l.to.id.slug}`);
    expect(links).toEqual(
      expect.arrayContaining([
        'storefront-checkout triggers add-item-to-cart',
        'add-item-to-cart emits cart.item-added',
        'cart.item-added updates cart-summary',
        'pending-payments observed by payment-capturer',
        'payment-capturer issues capture-payment',
        'order-status renders into order-status-page',
      ]),
    );
    expect(links).toHaveLength(14);
    expect(new Set(links).size).toBe(links.length);
  });

  it('links a projection declared only in source_events, as the Board does', () => {
    const { model } = build([
      { event: { id: id('order.placed'), title: 'Order placed' } },
      { readModel: { id: id('orders'), title: 'Orders', sourceEvents: [ref('order.placed')] } },
    ]);
    const [link] = happensNextLinks(model);
    expect(link.relation).toBe('updates');
    expect(link.slice).toBeUndefined();
  });

  it('marks a projection across namespaces as feeding another context', () => {
    const { model } = build([
      { event: { id: id('order.delivered', 'acme-storefront'), title: 'Order delivered' } },
      { readModel: { id: id('eligible-reviews', 'acme-reviews'), title: 'Eligible reviews' } },
      {
        readModelSlice: {
          id: id('eligible-reviews-rms', 'acme-reviews'),
          title: 'Eligible reviews',
          readModel: { readModel: ref('eligible-reviews', 'acme-reviews') },
          sourceEvents: [{ event: ref('order.delivered', 'acme-storefront') }],
        },
      },
    ]);
    expect(happensNextLinks(model).map((l) => l.relation)).toEqual(['feeds context']);
  });
});

describe('buildWaterfall: by storyboard', () => {
  it('numbers the header columns in orderedSlices order', () => {
    const { wf } = build(storefront());
    expect(wf.slices.map((s) => s.entity.id.slug)).toEqual([
      'add-item-to-cart-slice',
      'cart-summary-rms',
      'place-order-slice',
      'pending-payments-rms',
      'order-status-placed-rms',
      'capture-payment-auto',
      'capture-payment-slice',
      'order-status-paid-rms',
      'order-status-ui',
      'issue-refund-slice',
      'order-status-refunded-rms',
    ]);
  });

  it('walks each storyboard in its slice order, one row per step', () => {
    const { wf } = build(storefront());
    expect(wf.storyboards.map((g) => g.title)).toEqual(['Checkout', 'Payment', NOT_IN_A_STORYBOARD]);
    expect(describeRows(group(wf.storyboards, 'Checkout'))).toEqual([
      'step:storefront-checkout',
      'step:add-item-to-cart',
      'step:cart.item-added',
      'step:cart-summary',
      'above:storefront-checkout',
      'step:place-order',
      'step:order.placed',
      'step:pending-payments',
      'step:order-status',
    ]);
  });

  it('gives a repeat occurrence in the same chain an above row that points to the first one', () => {
    const { wf } = build(storefront());
    const checkout = group(wf.storyboards, 'Checkout');
    const above = checkout.rows[4];
    expect(above.type).toBe('above');
    expect(above.target).toEqual({ group: checkout.key, row: checkout.rows[0].key });
    expect(above.cols).toEqual([2]);
  });

  it('lets the first chain own a shared entity and gives later chains a join row', () => {
    const { wf } = build(storefront());
    const checkout = group(wf.storyboards, 'Checkout');
    const payment = group(wf.storyboards, 'Payment');
    expect(describeRows(payment)).toEqual([
      'join:pending-payments',
      'step:payment-capturer',
      'step:capture-payment',
      'step:payment.captured',
      'join:order-status',
      'step:order-status-page',
    ]);
    const join = payment.rows[0];
    const owner = checkout.rows.find((r) => r.entity.id.slug === 'pending-payments');
    expect(join.target).toEqual({ group: checkout.key, row: owner?.key });
    expect(join.targetTitle).toBe('Checkout');
    expect(owner?.joinedBy).toEqual(['Payment']);
    expect(wf.storyboardHome.get(join.entity.key)).toEqual({ group: checkout.key, row: owner?.key });
  });

  it('does not repeat a command the automation right before it issued', () => {
    const { wf } = build(storefront());
    const payment = group(wf.storyboards, 'Payment');
    expect(payment.rows.filter((r) => r.entity.id.slug === 'capture-payment')).toHaveLength(1);
  });

  it('puts entities no storyboard reaches in a final group, depth first along links', () => {
    const { wf } = build(storefront());
    const rest = group(wf.storyboards, NOT_IN_A_STORYBOARD);
    expect(rest.rows.map((r) => `${r.type}:${r.entity.id.slug}@${r.depth}`)).toEqual([
      'step:issue-refund@0',
      'step:refund.issued@1',
      'join:order-status@2',
      'step:order.archived@0',
    ]);
    // The join marks the slice that declares the link, not the owner's slice.
    expect(rest.rows[2].cols).toEqual([10]);
    // An entity in no slice has no column to mark.
    expect(rest.rows[3].cols).toEqual([]);
    const owner = group(wf.storyboards, 'Checkout').rows.find((r) => r.entity.id.slug === 'order-status');
    expect(owner?.joinedBy).toEqual(['Payment', NOT_IN_A_STORYBOARD]);
    expect(wf.shared).toBe(2);
  });

  it('starts a loop that has no way in at its earliest step', () => {
    const { wf } = build([
      { readModel: { id: id('stock-levels'), title: 'Stock levels' } },
      { processor: { id: id('reorderer'), title: 'Reorderer' } },
      { command: { id: id('reorder'), title: 'Reorder' } },
      { event: { id: id('supply.reordered'), title: 'Supply reordered' } },
      {
        automationSlice: {
          id: id('reorder-auto'),
          title: 'Reorder',
          processor: { processor: ref('reorderer') },
          emittedCommand: { command: ref('reorder') },
          sourceReadModels: [{ readModel: ref('stock-levels') }],
        },
      },
      {
        commandSlice: {
          id: id('reorder-slice'),
          title: 'Reorder',
          command: { command: ref('reorder') },
          emittedEvents: [{ event: ref('supply.reordered') }],
        },
      },
      {
        readModelSlice: {
          id: id('stock-levels-rms'),
          title: 'Stock levels',
          readModel: { readModel: ref('stock-levels') },
          sourceEvents: [{ event: ref('supply.reordered') }],
        },
      },
    ]);
    const [rest] = wf.storyboards;
    expect(rest.title).toBe(NOT_IN_A_STORYBOARD);
    // Slice 0 holds the read, the automation and its command. The command
    // leads by flow order, and the loop closes back on it.
    expect(describeRows(rest)).toEqual([
      'step:reorder',
      'step:supply.reordered',
      'step:stock-levels',
      'step:reorderer',
      'above:reorder',
    ]);
    const levels = rest.rows.find((r) => r.entity.id.slug === 'stock-levels');
    expect(wf.occurrences.get(levels?.entity.key ?? '')).toEqual([0, 2]);
  });

  it('shows a storyboard with no slices as an empty chain', () => {
    const { wf } = build([{ storyboard: { id: id('returns'), title: 'Returns', slices: [] } }]);
    expect(wf.storyboards).toHaveLength(1);
    expect(wf.storyboards[0].rows).toEqual([]);
    expect(wf.storyboards[0].entity?.id.slug).toBe('returns');
  });

  it('runs a bar from an occurrence to the last slice that reads it before the next one', () => {
    const { wf } = build(storefront());
    const checkout = group(wf.storyboards, 'Checkout');
    const span = (slug: string) => checkout.rows.find((r) => r.type === 'step' && r.entity.id.slug === slug)?.span;
    expect(span('order.placed')).toEqual({ start: 2, end: 4 });
    // Order status is projected again in slice 7, so this state ends here.
    expect(span('order-status')).toEqual({ start: 4, end: 4 });
    expect(checkout.span).toEqual({ start: 0, end: 4 });
  });

  it('summarizes each chain column by kind for the group header', () => {
    const { wf } = build(storefront());
    const col0 = group(wf.storyboards, 'Checkout').columns[0];
    expect(col0).toEqual({
      col: 0,
      total: 3,
      kinds: [
        { kind: 'ui', count: 1 },
        { kind: 'command', count: 1 },
        { kind: 'event', count: 1 },
      ],
    });
  });

  it('shows only the latest version of an entity', () => {
    const entities = storefront();
    entities.push({ command: { id: id('place-order', NS, '2'), title: 'Place order v2' } });
    const { wf } = build(entities);
    const rows = wf.storyboards.flatMap((g) => g.rows).filter((r) => r.entity.id.slug === 'place-order');
    expect(rows.map((r) => r.entity.id.version)).toEqual(['2']);
    expect(wf.types.flatMap((g) => g.members).filter((e) => e.id.slug === 'place-order')).toHaveLength(1);
  });

  it('gives each loaded storyboard version its own group and row keys', () => {
    const entities = storefront();
    entities.push({
      storyboard: {
        id: id('checkout-flow', NS, '2'),
        title: 'Checkout v2',
        slices: [ref('add-item-to-cart-slice'), ref('place-order-slice')],
      },
    });
    const { wf } = build(entities);
    const versions = wf.storyboards.filter((g) => g.entity?.id.slug === 'checkout-flow');
    expect(versions.map((g) => g.entity?.id.version)).toEqual(['1', '2']);
    expect(versions[0].key).not.toBe(versions[1].key);
    const rowKeys = wf.storyboards.flatMap((g) => g.rows.map((r) => r.key));
    expect(new Set(rowKeys).size).toBe(rowKeys.length);
  });
});

describe('buildWaterfall: by object type', () => {
  it('groups by Board lane and kind, in Board lane order and flow kind order', () => {
    const { wf } = build(storefront());
    expect(wf.types.map((g) => `${g.lane}/${g.title}`)).toEqual([
      'Automation/Automations',
      'Shopper/UIs',
      'Timeline/Commands',
      'Timeline/Read models',
      'Cart/Events · Cart',
      'Order/Events · Order',
    ]);
    expect(group(wf.types, 'Commands').members.map((e) => e.id.slug)).toEqual([
      'add-item-to-cart',
      'place-order',
      'capture-payment',
      'issue-refund',
    ]);
    expect(group(wf.types, 'Events · Order').members.map((e) => e.id.slug)).toEqual([
      'order.placed',
      'payment.captured',
      'refund.issued',
      'order.archived',
    ]);
  });

  it('names the lane in the title when one kind fills several groups', () => {
    const { wf } = build(storefront());
    const titles = wf.types.map((g) => g.title);
    expect(titles.filter((t) => t.startsWith('Events'))).toEqual(['Events · Cart', 'Events · Order']);
    expect(titles).toContain('Commands');
  });

  it('marks every slice where a member occurs', () => {
    const { wf } = build(storefront());
    const status = group(wf.types, 'Read models').rows.find((r) => r.entity.id.slug === 'order-status');
    expect(status?.cols).toEqual([4, 7, 10]);
    expect(status?.span).toEqual({ start: 4, end: 10 });
    expect(wf.occurrences.get(status?.entity.key ?? '')).toEqual([4, 7, 10]);
  });

  it('draws one cable per pair of linked groups, with the link count', () => {
    const { wf } = build(storefront());
    const title = (key: string) => {
      const g = wf.types.find((x) => x.key === key);
      return `${g?.lane}/${g?.title}`;
    };
    const cables = Object.fromEntries(wf.cables.map((c) => [`${title(c.from)} > ${title(c.to)}`, c.links.length]));
    expect(cables).toEqual({
      'Order/Events · Order > Timeline/Read models': 4,
      'Timeline/Commands > Order/Events · Order': 3,
      'Shopper/UIs > Timeline/Commands': 2,
      'Automation/Automations > Timeline/Commands': 1,
      'Cart/Events · Cart > Timeline/Read models': 1,
      'Timeline/Commands > Cart/Events · Cart': 1,
      'Timeline/Read models > Automation/Automations': 1,
      'Timeline/Read models > Shopper/UIs': 1,
    });
    expect(wf.cables.reduce((n, c) => n + c.links.length, 0)).toBe(wf.links.length);
    // Biggest cable first.
    expect(wf.cables[0].links).toHaveLength(4);
  });

  it('colors a cable by the flow it carries', () => {
    const { wf } = build(storefront());
    const byKinds = (from: string, to: string) =>
      wf.cables.find((c) => c.links[0].from.kind === from && c.links[0].to.kind === to)?.color;
    expect(byKinds('ui', 'command')).toBe('#0ea5e9');
    expect(byKinds('command', 'event')).toBe('#f97316');
    expect(byKinds('readModel', 'processor')).toBe('#10b981');
  });

  it('counts links in and out per group and per row', () => {
    const { wf } = build(storefront());
    const rms = group(wf.types, 'Read models');
    expect(rms.linksIn).toBe(5);
    expect(rms.linksOut).toBe(2);
    const status = rms.rows.find((r) => r.entity.id.slug === 'order-status');
    expect([status?.linksIn, status?.linksOut]).toEqual([3, 1]);
  });

  it('names lanes with their namespace when several namespaces are on screen', () => {
    const { wf } = build([
      { command: { id: id('place-order', 'acme-storefront'), title: 'Place order' } },
      { command: { id: id('submit-review', 'acme-reviews'), title: 'Submit review' } },
    ]);
    expect(wf.types.map((g) => g.lane)).toEqual(['acme-reviews · Timeline', 'acme-storefront · Timeline']);
  });
});

describe('code bundles', () => {
  it('bundles references by repo and the first two folders, most referenced first', () => {
    const { model } = build(storefront({ code: true }));
    const placeOrder = model.entities.find((e) => e.id.slug === 'place-order');
    const capturer = model.entities.find((e) => e.id.slug === 'payment-capturer');
    expect(codeBundles([placeOrder, capturer].filter((e) => e !== undefined))).toEqual([
      {
        key: 'storefront:services/checkout',
        repo: 'storefront',
        folder: 'services/checkout',
        label: 'storefront/services/checkout',
        refs: 2,
        files: ['services/checkout/handlers/http.go', 'services/checkout/place_order.go'],
      },
      { key: 'payments:', repo: 'payments', folder: '', label: 'payments', refs: 1, files: ['main.go'] },
      {
        key: 'payments:workers',
        repo: 'payments',
        folder: 'workers',
        label: 'payments/workers',
        refs: 1,
        files: ['workers/capture.go'],
      },
    ]);
  });

  it('puts bundles on step rows and rolls them up on group headers', () => {
    const { wf } = build(storefront({ code: true }));
    expect(wf.hasCode).toBe(true);
    const checkout = group(wf.storyboards, 'Checkout');
    const placeOrder = checkout.rows.find((r) => r.entity.id.slug === 'place-order');
    expect(placeOrder?.bundles.map((b) => b.label)).toEqual(['storefront/services/checkout']);
    expect(checkout.bundles.map((b) => [b.label, b.refs])).toEqual([
      ['storefront/services/checkout', 2],
      ['storefront/services/orders', 1],
    ]);
    // A join row points at the owner; the owner carries the code.
    const join = group(wf.storyboards, 'Payment').rows.find((r) => r.type === 'join');
    expect(join?.bundles).toEqual([]);
    expect(group(wf.types, 'Read models').bundles.map((b) => b.label)).toEqual(['storefront/services/orders']);
  });

  it('skips incomplete references', () => {
    const { model, wf } = build([
      { command: { id: id('place-order'), title: 'Place order', metadata: [codeRef('storefront', '')] } },
    ]);
    expect(codeBundles(model.entities)).toEqual([]);
    expect(wf.hasCode).toBe(false);
  });

  it('reports no code when no entity carries a reference', () => {
    const { wf } = build(storefront());
    expect(wf.hasCode).toBe(false);
    expect(wf.storyboards.every((g) => g.bundles.length === 0)).toBe(true);
    expect(wf.types.flatMap((g) => g.rows).every((r) => r.bundles.length === 0)).toBe(true);
  });
});

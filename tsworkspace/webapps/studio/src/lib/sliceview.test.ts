import { describe, expect, it } from 'vitest';
import { orderedSlices } from './model';
import {
  boardFieldNames,
  fieldFlow,
  type SliceNeighborhood,
  scenarioStepKeys,
  scenarioSteps,
  sliceBoardGeometry,
  sliceByParam,
  sliceForEntity,
  sliceGroups,
  sliceNeighborhood,
  sliceParam,
  storyboardsOf,
  storySteps,
  SLICE_CARD_H,
  SLICE_CARD_W,
} from './sliceview';
import { id, sliceNamed, storefront } from './sliceview.fixture';

// Cards as "<role>:<kind>:<slug>" per step, so a failing case reads like the board.
function stepsOf(hood: SliceNeighborhood): string[][] {
  return hood.steps.map((step) =>
    hood.cards.filter((c) => c.step === step.index).map((c) => `${step.role}:${c.kind}:${c.ref.slug}`),
  );
}

function linksOf(hood: SliceNeighborhood): string[] {
  const slugOf = (cardId: string) => hood.cards.find((c) => c.id === cardId)?.ref.slug;
  return hood.links.map((l) => `${slugOf(l.source)}->${slugOf(l.target)}`);
}

function linkId(hood: SliceNeighborhood, from: string, to: string): string {
  const link = hood.links.find(
    (l) =>
      hood.cards.find((c) => c.id === l.source)?.ref.slug === from &&
      hood.cards.find((c) => c.id === l.target)?.ref.slug === to,
  );
  if (!link) throw new Error(`no link ${from} -> ${to}`);
  return link.id;
}

function cardId(hood: SliceNeighborhood, slug: string): string {
  const card = hood.cards.find((c) => c.ref.slug === slug);
  if (!card) throw new Error(`no card ${slug}`);
  return card.id;
}

describe('sliceNeighborhood', () => {
  it('puts what feeds an automation slice before it and what it feeds after it', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'capture-payment-auto'));
    expect(stepsOf(hood)).toEqual([
      ['upstream:event:order.placed'],
      ['slice:readModel:pending-payments'],
      ['slice:processor:payment-capture-worker'],
      ['slice:command:capture-payment'],
      ['downstream:event:payment.captured'],
    ]);
    expect(linksOf(hood)).toEqual([
      'order.placed->pending-payments',
      'pending-payments->payment-capture-worker',
      'payment-capture-worker->capture-payment',
      'capture-payment->payment.captured',
    ]);
  });

  it('records the slice that declares each neighbor and each link', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'capture-payment-auto'));
    const byslug = (slug: string) => hood.cards.find((c) => c.ref.slug === slug);
    expect(byslug('order.placed')?.moment.id.slug).toBe('pending-payments-rms');
    expect(byslug('pending-payments')?.moment.id.slug).toBe('capture-payment-auto');
    expect(byslug('payment.captured')?.moment.id.slug).toBe('capture-payment-slice');
    expect(hood.links.map((l) => l.slice.id.slug)).toEqual([
      'pending-payments-rms',
      'capture-payment-auto',
      'capture-payment-auto',
      'capture-payment-slice',
    ]);
  });

  it('orders lanes like the board and gives each stream its own lane', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'capture-payment-auto'));
    expect(hood.lanes.map((l) => l.label)).toEqual(['Automation', 'Timeline', 'Order']);
    expect(hood.lanes[2].entity?.id.slug).toBe('order');
    expect(hood.cards.find((c) => c.kind === 'event')?.lane).toBe(hood.lanes[2].id);
  });

  it('fans a command slice out into every read model that projects its events', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'place-order-slice'));
    // Nothing on this model renders onto the checkout screen, so no step feeds it.
    expect(stepsOf(hood)).toEqual([
      ['slice:ui:storefront-checkout'],
      ['slice:command:place-order'],
      ['slice:event:order.placed'],
      ['downstream:readModel:pending-payments', 'downstream:readModel:order-status'],
    ]);
    expect(hood.lanes.map((l) => l.label)).toEqual(['UI', 'Timeline', 'Order']);
  });

  it('shows a command an automation issues as fed by that processor', () => {
    const model = storefront();
    const hood = sliceNamed(model, 'capture-payment-slice');
    expect(stepsOf(sliceNeighborhood(model, hood))).toEqual([
      ['upstream:processor:payment-capture-worker'],
      ['slice:command:capture-payment'],
      ['slice:event:payment.captured'],
      ['downstream:readModel:order-status'],
    ]);
  });

  it('collects every event projected into a screen and treats the screen as an end', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'order-tracking-ui-slice'));
    expect(stepsOf(hood)).toEqual([
      ['upstream:event:order.placed', 'upstream:event:payment.captured'],
      ['slice:readModel:order-status'],
      ['slice:ui:order-tracking'],
    ]);
  });

  it('lists a shared neighbor once with one link per connection', () => {
    const model = storefront([
      {
        readModelSlice: {
          id: id('order-status-placed-again-rms'),
          title: 'Order status again',
          readModel: { readModel: { id: id('order-status') } },
          sourceEvents: [{ event: { id: id('order.placed') } }],
        },
      },
    ]);
    const hood = sliceNeighborhood(model, sliceNamed(model, 'order-tracking-ui-slice'));
    expect(hood.cards.filter((c) => c.ref.slug === 'order.placed')).toHaveLength(1);
    expect(linksOf(hood).filter((l) => l === 'order.placed->order-status')).toHaveLength(1);
  });

  it('does not read another automation slice of the same processor as input', () => {
    const model = storefront([
      {
        automationSlice: {
          id: id('retry-capture-auto'),
          title: 'Retry capture',
          processor: { processor: { id: id('payment-capture-worker') } },
        },
      },
    ]);
    const hood = sliceNeighborhood(model, sliceNamed(model, 'retry-capture-auto'));
    expect(stepsOf(hood)).toEqual([['slice:processor:payment-capture-worker']]);
    expect(hood.links).toEqual([]);
  });

  it('ignores an older version of the same slice', () => {
    const model = storefront([
      {
        readModelSlice: {
          id: id('pending-payments-rms', '2'),
          title: 'Pending payments v2',
          readModel: { readModel: { id: id('pending-payments') } },
          sourceEvents: [{ event: { id: id('order.placed') } }],
        },
      },
    ]);
    const latest = model.slices.find((s) => s.entity.id.slug === 'pending-payments-rms' && s.entity.id.version === '2');
    if (!latest) throw new Error('missing v2');
    const hood = sliceNeighborhood(model, latest);
    expect(stepsOf(hood)[0]).toEqual(['upstream:command:place-order']);
    expect(hood.cards.filter((c) => c.role === 'upstream')).toHaveLength(1);
  });

  it('shows the loaded entity when the slice pins an older version', () => {
    const model = storefront([
      {
        command: {
          id: id('place-order', '2'),
          title: 'Place order v2',
          schema: {
            '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema',
            fields: [
              { name: 'order_id', type: { uuid: {} } },
              { name: 'cart_id', type: { uuid: {} } },
              { name: 'buyer_id', type: { uuid: {} } },
            ],
          },
        },
      },
    ]);
    const hood = sliceNeighborhood(model, sliceNamed(model, 'place-order-slice'));
    const command = hood.cards.find((c) => c.ref.slug === 'place-order');
    expect(command?.ref.version).toBe('1');
    expect(command?.entity?.id.version).toBe('2');
    expect(command?.entity?.title).toBe('Place order v2');
  });

  it('keeps an entity the slice names but the model does not load', () => {
    const model = storefront([
      {
        readModelSlice: {
          id: id('refunds-rms'),
          title: 'Refunds',
          readModel: { readModel: { id: id('refunds') } },
          sourceEvents: [{ event: { id: { namespace: 'acme-payments', slug: 'refund.issued', version: '1' } } }],
        },
      },
    ]);
    const hood = sliceNeighborhood(model, sliceNamed(model, 'refunds-rms'));
    expect(hood.cards.map((c) => [c.ref.slug, c.entity === undefined, c.lane])).toEqual([
      ['refund.issued', true, 'events'],
      ['refunds', true, 'timeline'],
    ]);
    expect(hood.lanes.map((l) => l.label)).toEqual(['Timeline', 'Events']);
    expect(hood.links).toHaveLength(1);
  });

  it('draws one stream lane per swimlane in swimlane order', () => {
    const model = storefront([
      {
        commandSlice: {
          id: id('checkout-everything-slice'),
          title: 'Two streams',
          command: { command: { id: id('place-order') } },
          emittedEvents: [{ event: { id: id('order.placed') } }, { event: { id: id('cart.item-added') } }],
        },
      },
    ]);
    const hood = sliceNeighborhood(model, sliceNamed(model, 'checkout-everything-slice'));
    expect(hood.lanes.map((l) => l.label).filter((l) => l === 'Cart' || l === 'Order')).toEqual(['Cart', 'Order']);
  });
});

describe('sliceBoardGeometry', () => {
  it('places every card inside its lane without overlap and links left to right', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'place-order-slice'));
    const geo = sliceBoardGeometry(hood);
    const boxes = hood.cards.map((c) => ({ c, p: geo.cards.get(c.id) }));
    for (const { c, p } of boxes) {
      if (!p) throw new Error(`no position for ${c.id}`);
      const lane = geo.lanes.find((l) => l.lane.id === c.lane);
      if (!lane) throw new Error(`no lane for ${c.id}`);
      expect(p.y).toBeGreaterThanOrEqual(lane.y);
      expect(p.y + SLICE_CARD_H).toBeLessThanOrEqual(lane.y + lane.height);
      expect(p.x + SLICE_CARD_W).toBeLessThanOrEqual(geo.width);
    }
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const a = boxes[i].p;
        const b = boxes[j].p;
        if (!a || !b) continue;
        const apart =
          a.x + SLICE_CARD_W <= b.x ||
          b.x + SLICE_CARD_W <= a.x ||
          a.y + SLICE_CARD_H <= b.y ||
          b.y + SLICE_CARD_H <= a.y;
        expect(apart).toBe(true);
      }
    }
    for (const l of hood.links) {
      const s = geo.cards.get(l.source);
      const t = geo.cards.get(l.target);
      if (!s || !t) throw new Error(`unplaced link ${l.id}`);
      expect(t.x).toBeGreaterThan(s.x + SLICE_CARD_W);
      const path = geo.links.get(l.id);
      expect(path?.d.startsWith(`M${s.x + SLICE_CARD_W},`)).toBe(true);
      expect(path?.mid.x).toBeGreaterThan(s.x + SLICE_CARD_W);
      expect(path?.mid.x).toBeLessThan(t.x);
    }
  });

  it('stacks two cards of one lane and step and grows the lane to fit', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'place-order-slice'));
    const geo = sliceBoardGeometry(hood);
    const pending = geo.cards.get(cardId(hood, 'pending-payments'));
    const status = geo.cards.get(cardId(hood, 'order-status'));
    expect(pending?.x).toBe(status?.x);
    expect((status?.y ?? 0) - (pending?.y ?? 0)).toBeGreaterThanOrEqual(SLICE_CARD_H);
    const timeline = geo.lanes.find((l) => l.lane.id === 'timeline');
    expect(timeline?.height).toBeGreaterThan(2 * SLICE_CARD_H);
  });
});

describe('fieldFlow', () => {
  it('carries a field through every card of the chain when each one has it', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'capture-payment-auto'));
    const flow = fieldFlow(model, hood, 'order_id');
    expect(hood.cards.map((c) => flow.presence.get(c.id))).toEqual([
      'carries',
      'carries',
      'passes',
      'carries',
      'carries',
    ]);
    expect(flow.carried.size).toBe(hood.links.length);
    expect(flow.stops.size).toBe(0);
  });

  it('marks the link where a field stops and leaves the rest of the chain dark', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'pending-payments-rms'));
    const flow = fieldFlow(model, hood, 'cart_id');
    expect(flow.presence.get(cardId(hood, 'place-order'))).toBe('carries');
    expect(flow.presence.get(cardId(hood, 'order.placed'))).toBe('carries');
    expect(flow.presence.get(cardId(hood, 'pending-payments'))).toBe('lacks');
    // The processor downstream reads a read model without the field.
    expect(flow.presence.get(cardId(hood, 'payment-capture-worker'))).toBe('none');
    expect([...flow.stops]).toEqual([linkId(hood, 'order.placed', 'pending-payments')]);
    expect([...flow.carried]).toEqual([linkId(hood, 'place-order', 'order.placed')]);
  });

  it('passes a field through a processor and stops at a command that does not take it', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'capture-payment-auto'));
    const flow = fieldFlow(model, hood, 'buyer_id');
    expect(flow.presence.get(cardId(hood, 'payment-capture-worker'))).toBe('passes');
    expect([...flow.stops]).toEqual([linkId(hood, 'payment-capture-worker', 'capture-payment')]);
  });

  it('marks the link into a processor when only the command it feeds holds the field', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'capture-payment-auto'));
    const flow = fieldFlow(model, hood, 'amount');
    expect(flow.presence.get(cardId(hood, 'pending-payments'))).toBe('lacks');
    expect(flow.presence.get(cardId(hood, 'capture-payment'))).toBe('carries');
    expect(flow.presence.get(cardId(hood, 'payment-capture-worker'))).toBe('none');
    expect([...flow.stops]).toEqual([linkId(hood, 'pending-payments', 'payment-capture-worker')]);
    expect([...flow.carried]).toEqual([linkId(hood, 'capture-payment', 'payment.captured')]);
  });

  it('does not judge the source of a neighbor slice processor whose read models are off the board', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'capture-payment-slice'));
    const flow = fieldFlow(model, hood, 'amount');
    expect(flow.presence.get(cardId(hood, 'capture-payment'))).toBe('carries');
    expect(flow.stops.has(linkId(hood, 'payment-capture-worker', 'capture-payment'))).toBe(false);
  });

  it('counts a field the event stream keeps as state, as the validator does', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'order-status-placed-rms'));
    const flow = fieldFlow(model, hood, 'status');
    expect(flow.presence.get(cardId(hood, 'order.placed'))).toBe('state');
    expect(flow.presence.get(cardId(hood, 'order-status'))).toBe('carries');
    expect(flow.carried.has(linkId(hood, 'order.placed', 'order-status'))).toBe(true);
    // The command before the event never carried the status.
    expect(flow.stops.size).toBe(0);
  });

  it('reports an entity that is not loaded as unloaded and marks no stop into it', () => {
    const model = storefront([
      {
        readModelSlice: {
          id: id('order-archive-rms'),
          title: 'Archive',
          readModel: { readModel: { id: id('order-archive') } },
          sourceEvents: [{ event: { id: id('order.placed') } }],
        },
      },
    ]);
    const hood = sliceNeighborhood(model, sliceNamed(model, 'order-archive-rms'));
    const flow = fieldFlow(model, hood, 'order_id');
    expect(flow.presence.get(cardId(hood, 'order-archive'))).toBe('unloaded');
    expect(flow.stops.size).toBe(0);
  });

  it('matches only along links, not by a shared name elsewhere on the model', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'cart-summary-rms'));
    // place-order also has cart_id, but nothing links it to this slice.
    expect(hood.cards.some((c) => c.ref.slug === 'place-order')).toBe(false);
    const flow = fieldFlow(model, hood, 'cart_id');
    expect([...flow.presence.values()].filter((p) => p === 'carries')).toHaveLength(3);
  });
});

describe('fieldFlow on a derived field', () => {
  const amountOnCapture = (doc: string) => {
    const model = storefront();
    const amount = model.byKey
      .get('command:acme-storefront/capture-payment@1')
      ?.fields.find((f) => f.name === 'amount');
    if (!amount) throw new Error('missing amount field');
    amount.metadata = [{ '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation', doc }];
    return model;
  };

  it('does not stop at a processor whose outbound field is derived with a doc', () => {
    const model = amountOnCapture('Computed from the order total.');
    const hood = sliceNeighborhood(model, sliceNamed(model, 'capture-payment-auto'));
    const flow = fieldFlow(model, hood, 'amount');
    expect(flow.presence.get(cardId(hood, 'capture-payment'))).toBe('carries');
    expect(flow.stops.has(linkId(hood, 'pending-payments', 'payment-capture-worker'))).toBe(false);
  });

  it('still stops when the derived annotation has an empty doc', () => {
    const model = amountOnCapture('');
    const hood = sliceNeighborhood(model, sliceNamed(model, 'capture-payment-auto'));
    const flow = fieldFlow(model, hood, 'amount');
    expect(flow.stops.has(linkId(hood, 'pending-payments', 'payment-capture-worker'))).toBe(true);
  });
});

describe('boardFieldNames', () => {
  it('lists each field name once in board order', () => {
    const model = storefront();
    const hood = sliceNeighborhood(model, sliceNamed(model, 'capture-payment-auto'));
    expect(boardFieldNames(hood)).toEqual(['order_id', 'cart_id', 'buyer_id', 'amount']);
  });
});

describe('picking a slice', () => {
  it('groups slices by storyboard in the same order as orderedSlices', () => {
    const model = storefront([
      { commandSlice: { id: id('zz-loose-slice'), title: 'Loose', command: { command: { id: id('place-order') } } } },
    ]);
    const groups = sliceGroups(model);
    expect(groups.map((g) => g.storyboard?.title ?? null)).toEqual(['Checkout', 'Fulfillment', null]);
    expect(groups.flatMap((g) => g.slices.map((s) => s.entity.key))).toEqual(
      orderedSlices(model).map((s) => s.entity.key),
    );
  });

  it('lists a slice two storyboards share once, under the first', () => {
    const model = storefront([
      { storyboard: { id: id('reorder-flow'), title: 'Reorder', slices: [{ id: id('place-order-slice') }] } },
    ]);
    const groups = sliceGroups(model);
    const holders = groups.filter((g) => g.slices.some((s) => s.entity.id.slug === 'place-order-slice'));
    expect(holders.map((g) => g.storyboard?.title)).toEqual(['Checkout']);
  });

  it('round-trips the URL value', () => {
    const model = storefront();
    const slice = sliceNamed(model, 'capture-payment-auto');
    expect(sliceParam(slice)).toBe('acme-storefront/capture-payment-auto');
    expect(sliceByParam(model, sliceParam(slice))?.entity.key).toBe(slice.entity.key);
    expect(sliceByParam(model, 'acme-storefront/nope')).toBeUndefined();
    expect(sliceByParam(model, null)).toBeUndefined();
  });

  it('opens the selected slice, or the first slice that names the selected entity', () => {
    const model = storefront();
    expect(sliceForEntity(model, 'automationSlice:acme-storefront/capture-payment-auto@1')?.entity.id.slug).toBe(
      'capture-payment-auto',
    );
    expect(sliceForEntity(model, 'event:acme-storefront/order.placed@1')?.entity.id.slug).toBe('place-order-slice');
    expect(sliceForEntity(model, 'persona:acme-storefront/shopper@1')?.entity.id.slug).toBe('add-item-to-cart-slice');
    expect(sliceForEntity(model, 'swimlane:acme-storefront/order@1')).toBeUndefined();
    expect(sliceForEntity(model, undefined)).toBeUndefined();
  });
});

describe('a slice loaded at two versions', () => {
  const twoVersions = () =>
    storefront([
      {
        commandSlice: {
          id: id('place-order-slice', '2'),
          title: 'Place order v2',
          command: { command: { id: id('place-order') } },
        },
      },
      {
        commandSlice: {
          id: id('zz-loose-slice', '2'),
          title: 'Loose v2',
          command: { command: { id: id('place-order') } },
        },
      },
      {
        commandSlice: {
          id: id('zz-loose-slice', '1'),
          title: 'Loose v1',
          command: { command: { id: id('place-order') } },
        },
      },
    ]);

  it('uses the highest version in the picker, the URL and story playback, whatever the load order', () => {
    const model = twoVersions();
    const picked = sliceGroups(model).flatMap((g) => g.slices.filter((s) => s.entity.id.slug === 'place-order-slice'));
    expect(picked.map((s) => s.entity.id.version)).toEqual(['2']);
    expect(sliceByParam(model, 'acme-storefront/place-order-slice')?.entity.id.version).toBe('2');
    const checkout = model.storyboards.find((s) => s.title === 'Checkout');
    if (!checkout) throw new Error('missing storyboard');
    const played = storySteps(model, checkout).slices.filter((s) => s.entity.id.slug === 'place-order-slice');
    expect(played.map((s) => s.entity.id.version)).toEqual(['2']);
  });

  it('opens the latest version when focus names an older version of a slice', () => {
    const model = twoVersions();
    expect(sliceForEntity(model, 'commandSlice:acme-storefront/place-order-slice@1')?.entity.id.version).toBe('2');
  });

  it('lists one slice per slug on every view that shares orderedSlices', () => {
    const loose = orderedSlices(twoVersions()).filter((s) => s.entity.id.slug === 'zz-loose-slice');
    expect(loose.map((s) => s.entity.id.version)).toEqual(['2']);
  });
});

describe('story playback', () => {
  it('finds the storyboards that list a slice', () => {
    const model = storefront([
      { storyboard: { id: id('reorder-flow'), title: 'Reorder', slices: [{ id: id('place-order-slice') }] } },
    ]);
    expect(storyboardsOf(model, sliceNamed(model, 'place-order-slice')).map((s) => s.title)).toEqual([
      'Checkout',
      'Reorder',
    ]);
    expect(storyboardsOf(model, sliceNamed(model, 'capture-payment-auto')).map((s) => s.title)).toEqual([
      'Fulfillment',
    ]);
  });

  it('keeps storyboard order and repeated moments and reports what is not loaded', () => {
    const model = storefront([
      {
        storyboard: {
          id: id('retry-flow'),
          title: 'Retry',
          slices: [
            { id: id('capture-payment-auto') },
            { id: id('capture-payment-slice') },
            { id: id('missing-slice') },
            { id: id('capture-payment-auto') },
          ],
        },
      },
    ]);
    const sb = model.storyboards.find((s) => s.title === 'Retry');
    if (!sb) throw new Error('missing storyboard');
    const steps = storySteps(model, sb);
    expect(steps.slices.map((s) => s.entity.id.slug)).toEqual([
      'capture-payment-auto',
      'capture-payment-slice',
      'capture-payment-auto',
    ]);
    expect(steps.missing.map((m) => m.slug)).toEqual(['missing-slice']);
  });
});

describe('scenario playback', () => {
  it('steps a command scenario through given, when and then', () => {
    const model = storefront();
    const slice = sliceNamed(model, 'place-order-slice');
    const steps = scenarioSteps(slice, slice.scenarios[0]);
    expect(steps.map((s) => s.phase)).toEqual(['given', 'when', 'then']);
    expect(steps.map((s) => [...scenarioStepKeys(s)])).toEqual([
      ['event:acme-storefront/cart.item-added'],
      ['command:acme-storefront/place-order'],
      ['event:acme-storefront/order.placed'],
    ]);
    expect(steps[1].examples[0].data).toEqual({ cart_id: 'c-1' });
  });

  it('keeps an empty given and ends a rejected scenario on its reason', () => {
    const model = storefront();
    const slice = sliceNamed(model, 'place-order-slice');
    const steps = scenarioSteps(slice, slice.scenarios[1]);
    expect(steps.map((s) => s.phase)).toEqual(['given', 'when', 'then']);
    expect(steps[0].examples).toEqual([]);
    expect(steps[2].examples).toEqual([]);
    expect(steps[2].reject).toEqual({ reasonCode: 'EMPTY_CART', doc: 'A cart needs one item.' });
  });

  it('gives a read model scenario no given step and ends on the state', () => {
    const model = storefront();
    const slice = sliceNamed(model, 'cart-summary-rms');
    const steps = scenarioSteps(slice, slice.scenarios[0]);
    expect(steps.map((s) => s.phase)).toEqual(['when', 'then']);
    expect([...scenarioStepKeys(steps[1])]).toEqual(['readModel:acme-storefront/cart-summary']);
    expect(steps[1].examples[0].data).toEqual({ item_count: 1 });
  });

  it('carries an automation trigger written in prose and ends on the command', () => {
    const model = storefront();
    const slice = sliceNamed(model, 'capture-payment-auto');
    const steps = scenarioSteps(slice, slice.scenarios[0]);
    expect(steps.map((s) => s.phase)).toEqual(['given', 'when', 'then']);
    expect(steps[1]).toEqual({ phase: 'when', examples: [], doc: 'The payment provider confirms the capture.' });
    expect([...scenarioStepKeys(steps[2])]).toEqual(['command:acme-storefront/capture-payment']);
  });
});

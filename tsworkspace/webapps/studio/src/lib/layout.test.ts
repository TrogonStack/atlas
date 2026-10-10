// Smoke tests for the canvas layout engine. `layoutBoard` is too large
// to test exhaustively at the pass level today; these tests pin the
// contracts that have regressed at least once: produce SOME nodes for
// a non-empty model, produce edges only between resolvable refs, and
// keep occurrence columns monotonically non-decreasing on storyboard
// slice order.
import { describe, expect, it } from 'vitest';
import { layoutBoard, NODE_H } from './layout';
import { buildModel } from './model';

const id = (namespace: string, slug: string, version = '1'): Record<string, unknown> => ({
  namespace,
  slug,
  version,
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
      storyboard: {
        id: id(ns, 'sb-buy-flow'),
        title: 'buy flow',
        slices: [
          { id: id(ns, 's1-open') },
          { id: id(ns, 's2-view') },
          { id: id(ns, 's2-view-ui') },
          { id: id(ns, 's3-buy') },
        ],
      },
    },
  ];
}

function multiEmitEntities(): Record<string, unknown>[] {
  const ns = 'shop';
  return [
    { persona: { id: id(ns, 'buyer'), title: 'Buyer' } },
    { ui: { id: id(ns, 'checkout'), title: 'Checkout' } },
    { command: { id: id(ns, 'place-order'), title: 'Place order' } },
    { swimlane: { id: id(ns, 'order'), title: 'Order' } },
    { event: { id: id(ns, 'order.placed'), title: 'Order placed', swimlane: { id: id(ns, 'order') } } },
    { event: { id: id(ns, 'payment.requested'), title: 'Payment requested', swimlane: { id: id(ns, 'order') } } },
    {
      commandSlice: {
        id: id(ns, 's1-place-order'),
        title: 'place order',
        persona: { persona: { id: id(ns, 'buyer') } },
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
  ];
}

function stackedEventEntities(): Record<string, unknown>[] {
  const ns = 'shop';
  return [
    { ui: { id: id(ns, 'dashboard'), title: 'Dashboard' } },
    { event: { id: id(ns, 'inventory.reserved'), title: 'Inventory reserved' } },
    { event: { id: id(ns, 'payment.authorized'), title: 'Payment authorized' } },
    {
      readModel: {
        id: id(ns, 'order-status'),
        title: 'Order status',
        sourceEvents: [{ id: id(ns, 'inventory.reserved') }, { id: id(ns, 'payment.authorized') }],
      },
    },
    {
      readModelSlice: {
        id: id(ns, 's1-status'),
        title: 'status projects',
        readModel: { readModel: { id: id(ns, 'order-status') } },
        sourceEvents: [
          { event: { id: id(ns, 'inventory.reserved') } },
          { event: { id: id(ns, 'payment.authorized') } },
        ],
      },
    },
    {
      uiSlice: {
        id: id(ns, 's1-status-ui'),
        title: 'dashboard shows the status',
        sourceReadModels: [{ readModel: { id: id(ns, 'order-status') } }],
        ui: { ui: { id: id(ns, 'dashboard') } },
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-status'),
        title: 'status',
        slices: [{ id: id(ns, 's1-status') }, { id: id(ns, 's1-status-ui') }],
      },
    },
  ];
}

// Cross-namespace seam: an event in 'upstream' feeds a read model in 'shop'.
// The layout should produce a 'feeds context' edge across namespaces.
function crossNamespaceSeamEntities(): Record<string, unknown>[] {
  return [
    { event: { id: id('upstream', 'order.placed'), title: 'Order placed' } },
    {
      readModel: {
        id: id('shop', 'order-projection'),
        title: 'Order projection',
        sourceEvents: [{ id: id('upstream', 'order.placed') }],
      },
    },
    {
      readModelSlice: {
        id: id('shop', 's1-projection'),
        title: 'projection runs',
        readModel: { readModel: { id: id('shop', 'order-projection') } },
        sourceEvents: [{ event: { id: id('upstream', 'order.placed') } }],
      },
    },
    { ui: { id: id('shop', 'orders-ui'), title: 'Orders UI' } },
    {
      uiSlice: {
        id: id('shop', 's2-projection-ui'),
        title: 'orders UI shows the projection',
        sourceReadModels: [{ readModel: { id: id('shop', 'order-projection') } }],
        ui: { ui: { id: id('shop', 'orders-ui') } },
      },
    },
    {
      storyboard: {
        id: id('shop', 'sb-projection'),
        title: 'projection flow',
        slices: [{ id: id('shop', 's1-projection') }, { id: id('shop', 's2-projection-ui') }],
      },
    },
  ];
}

// Entity appearing in more than one storyboard: s1 is claimed by sb-a and sb-b.
function entityInMultipleStoryboardsEntities(): Record<string, unknown>[] {
  const ns = 'shop';
  return [
    { persona: { id: id(ns, 'buyer'), title: 'Buyer' } },
    { ui: { id: id(ns, 'home'), title: 'Home' } },
    { command: { id: id(ns, 'open-item'), title: 'open-item' } },
    { event: { id: id(ns, 'item.opened'), title: 'item.opened' } },
    {
      commandSlice: {
        id: id(ns, 's1-shared'),
        title: 'shared open',
        persona: { persona: { id: id(ns, 'buyer') } },
        ui: { ui: { id: id(ns, 'home') } },
        command: { command: { id: id(ns, 'open-item') } },
        emittedEvents: [{ event: { id: id(ns, 'item.opened') } }],
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-a'),
        title: 'flow A',
        slices: [{ id: id(ns, 's1-shared') }],
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-b'),
        title: 'flow B',
        slices: [{ id: id(ns, 's1-shared') }],
      },
    },
  ];
}

// Empty storyboard with an entry observer: storyboard has no slices listed,
// but its raw.entry declares a persona observer.
function emptyStoryboardWithEntryObserverEntities(): Record<string, unknown>[] {
  const ns = 'shop';
  return [
    { persona: { id: id(ns, 'buyer'), title: 'Buyer' } },
    {
      readModel: {
        id: id(ns, 'cart'),
        title: 'Cart',
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-empty'),
        title: 'empty entry',
        slices: [],
        entry: {
          human: {
            persona: { persona: { id: id(ns, 'buyer') } },
          },
          readModel: { readModel: { id: id(ns, 'cart') } },
        },
      },
    },
  ];
}

// A DISPLAY-ONLY screen: reached solely through a ReadModelSlice (which
// carries `ui` but no persona), so a CommandSlice can never attribute it.
// The storyboard's HumanObserver entry is the only place the model says who
// watches it: that must be enough to keep it out of `ui:unassigned`.
function displayOnlyUiWatchedByStoryboardEntities(): Record<string, unknown>[] {
  const ns = 'shop';
  return [
    { persona: { id: id(ns, 'manager'), title: 'Manager' } },
    { ui: { id: id(ns, 'ops-console'), title: 'Ops console' } },
    { event: { id: id(ns, 'order.placed'), title: 'Order placed' } },
    {
      readModel: {
        id: id(ns, 'orders-list'),
        title: 'Orders list',
        sourceEvents: [{ id: id(ns, 'order.placed') }],
      },
    },
    {
      readModelSlice: {
        id: id(ns, 's1-orders'),
        title: 'orders project',
        readModel: { readModel: { id: id(ns, 'orders-list') } },
        sourceEvents: [{ event: { id: id(ns, 'order.placed') } }],
      },
    },
    {
      uiSlice: {
        id: id(ns, 's2-orders-ui'),
        title: 'ops console shows the orders',
        sourceReadModels: [{ readModel: { id: id(ns, 'orders-list') } }],
        ui: { ui: { id: id(ns, 'ops-console') } },
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-ops'),
        title: 'ops watches orders',
        slices: [{ id: id(ns, 's1-orders') }, { id: id(ns, 's2-orders-ui') }],
        entry: {
          human: {
            persona: { persona: { id: id(ns, 'manager') } },
            ui: { ui: { id: id(ns, 'ops-console') } },
          },
          readModel: { readModel: { id: id(ns, 'orders-list') } },
        },
      },
    },
  ];
}

// Automation slice consuming several source read models: they should fan
// side-by-side (distinct x) rather than stack vertically in one column.
function multiSourceAutomationEntities(): Record<string, unknown>[] {
  const ns = 'shop';
  return [
    { readModel: { id: id(ns, 'rpc-requests'), title: 'RPC requests' } },
    { readModel: { id: id(ns, 'warehouse-rows'), title: 'Warehouse rows' } },
    { readModel: { id: id(ns, 'vendor-pending'), title: 'Vendor pending' } },
    { processor: { id: id(ns, 'intake'), title: 'Intake' } },
    { command: { id: id(ns, 'initiate'), title: 'Initiate' } },
    {
      automationSlice: {
        id: id(ns, 's1-intake'),
        title: 'intake',
        processor: { processor: { id: id(ns, 'intake') } },
        emittedCommand: { command: { id: id(ns, 'initiate') } },
        sourceReadModels: [
          { readModel: { id: id(ns, 'rpc-requests') } },
          { readModel: { id: id(ns, 'warehouse-rows') } },
          { readModel: { id: id(ns, 'vendor-pending') } },
        ],
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-intake'),
        title: 'intake flow',
        slices: [{ id: id(ns, 's1-intake') }],
      },
    },
  ];
}

// Automation slice whose only source read model already exists on the board
// (projected by an earlier rmSlice): the automation column must not reserve
// an empty base slot: its processor is the slice's leftmost occupant.
function reusedSourceAutomationEntities(): Record<string, unknown>[] {
  const ns = 'shop';
  return [
    { ui: { id: id(ns, 'dashboard'), title: 'Dashboard' } },
    { event: { id: id(ns, 'order.placed'), title: 'Order placed' } },
    { readModel: { id: id(ns, 'orders-feed'), title: 'Orders feed' } },
    { processor: { id: id(ns, 'fulfiller'), title: 'Fulfiller' } },
    { command: { id: id(ns, 'fulfill'), title: 'Fulfill' } },
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
        id: id(ns, 's1-feed-ui'),
        title: 'dashboard shows the feed',
        sourceReadModels: [{ readModel: { id: id(ns, 'orders-feed') } }],
        ui: { ui: { id: id(ns, 'dashboard') } },
      },
    },
    { event: { id: id(ns, 'order.fulfilled'), title: 'Order fulfilled' } },
    {
      automationSlice: {
        id: id(ns, 's2-fulfill-auto'),
        title: 'auto fulfill',
        processor: { processor: { id: id(ns, 'fulfiller') } },
        emittedCommand: { command: { id: id(ns, 'fulfill') } },
        sourceReadModels: [{ readModel: { id: id(ns, 'orders-feed') } }],
      },
    },
    {
      commandSlice: {
        id: id(ns, 's3-fulfill'),
        title: 'fulfill',
        command: { command: { id: id(ns, 'fulfill') } },
        emittedEvents: [{ event: { id: id(ns, 'order.fulfilled') } }],
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-fulfill'),
        title: 'fulfill flow',
        slices: [{ id: id(ns, 's1-feed') }, { id: id(ns, 's2-fulfill-auto') }, { id: id(ns, 's3-fulfill') }],
      },
    },
  ];
}

// The catch-all lanes once lived under a reserved `__unassigned__` KEY in the
// same map as the real lanes, and nothing stopped a modeler from naming a
// persona or a swimlane `__unassigned__` and shadowing it. `LaneIndex` holds
// the catch-all in a field of its own, so this model must yield FOUR distinct
// lanes: the reserved-slug persona and swimlane, plus both catch-alls.
function reservedSlugEntities(): Record<string, unknown>[] {
  const ns = 'shop';
  const reserved = '__unassigned__';
  return [
    { persona: { id: id(ns, reserved), title: 'Reserved persona' } },
    { swimlane: { id: id(ns, reserved), title: 'Reserved swimlane' } },
    { ui: { id: id(ns, 'owned'), title: 'Owned' } },
    { ui: { id: id(ns, 'orphan'), title: 'Orphan' } },
    { command: { id: id(ns, 'act'), title: 'act' } },
    { event: { id: id(ns, 'on-lane'), title: 'On lane', swimlane: { id: id(ns, reserved) } } },
    { event: { id: id(ns, 'off-lane'), title: 'Off lane' } },
    {
      readModel: {
        id: id(ns, 'view'),
        title: 'View',
        sourceEvents: [{ id: id(ns, 'on-lane') }],
      },
    },
    {
      commandSlice: {
        id: id(ns, 's1-act'),
        title: 'act',
        persona: { persona: { id: id(ns, reserved) } },
        ui: { ui: { id: id(ns, 'owned') } },
        command: { command: { id: id(ns, 'act') } },
        emittedEvents: [{ event: { id: id(ns, 'on-lane') } }, { event: { id: id(ns, 'off-lane') } }],
      },
    },
    {
      readModelSlice: {
        id: id(ns, 's2-view'),
        title: 'view projects',
        readModel: { readModel: { id: id(ns, 'view') } },
        sourceEvents: [{ event: { id: id(ns, 'on-lane') } }],
      },
    },
    {
      uiSlice: {
        id: id(ns, 's3-view-ui'),
        title: 'orphan shows the view',
        sourceReadModels: [{ readModel: { id: id(ns, 'view') } }],
        ui: { ui: { id: id(ns, 'orphan') } },
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-act'),
        title: 'act',
        slices: [{ id: id(ns, 's1-act') }, { id: id(ns, 's2-view') }, { id: id(ns, 's3-view-ui') }],
      },
    },
  ];
}

// A display-only screen with no command slice and no storyboard entry: the
// UI slice's own persona is the only thing that can attribute it.
function displayOnlyUiOwnedByUiSliceEntities(): Record<string, unknown>[] {
  const ns = 'ops';
  return [
    { persona: { id: id(ns, 'manager'), title: 'Manager' } },
    { ui: { id: id(ns, 'ops-console'), title: 'Ops console' } },
    { command: { id: id(ns, 'place-order'), title: 'Place order' } },
    { event: { id: id(ns, 'order.placed'), title: 'Order placed' } },
    { readModel: { id: id(ns, 'orders-list'), title: 'Orders list' } },
    {
      commandSlice: {
        id: id(ns, 's1-place'),
        title: 'place an order',
        command: { command: { id: id(ns, 'place-order') } },
        emittedEvents: [{ event: { id: id(ns, 'order.placed') } }],
      },
    },
    {
      readModelSlice: {
        id: id(ns, 's2-orders'),
        title: 'project the orders list',
        readModel: { readModel: { id: id(ns, 'orders-list') } },
        sourceEvents: [{ event: { id: id(ns, 'order.placed') } }],
      },
    },
    {
      uiSlice: {
        id: id(ns, 's3-orders-ui'),
        title: 'the console shows the orders',
        persona: { persona: { id: id(ns, 'manager') } },
        sourceReadModels: [{ readModel: { id: id(ns, 'orders-list') } }],
        ui: { ui: { id: id(ns, 'ops-console') } },
      },
    },
    {
      storyboard: {
        id: id(ns, 'sb-orders'),
        title: 'ops watches orders',
        slices: [{ id: id(ns, 's1-place') }, { id: id(ns, 's2-orders') }, { id: id(ns, 's3-orders-ui') }],
      },
    },
  ];
}

describe('layoutBoard', () => {
  it('produces a non-empty graph for a non-empty model', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes, edges } = layoutBoard(model);
    expect(nodes.length).toBeGreaterThan(0);
    expect(edges.length).toBeGreaterThan(0);
  });

  it('does not crash on an empty model', () => {
    const model = buildModel([]);
    expect(() => layoutBoard(model)).not.toThrow();
  });

  it('only emits edges whose endpoints exist in the node set', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes, edges } = layoutBoard(model);
    const ids = new Set(nodes.map((n) => n.id));
    for (const e of edges) {
      expect(ids.has(e.source), `dangling source ${e.source} on edge ${e.id}`).toBe(true);
      expect(ids.has(e.target), `dangling target ${e.target} on edge ${e.id}`).toBe(true);
    }
  });

  it('places nodes across distinct x columns (the board has a time axis)', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes } = layoutBoard(model);
    const distinctX = new Set(nodes.map((n) => Math.round(n.position.x)));
    expect(distinctX.size, 'all nodes collapsed to one column').toBeGreaterThan(1);
  });

  it('connects a command slice to every event it emits', () => {
    const model = buildModel(multiEmitEntities() as never);
    const { nodes, edges } = layoutBoard(model);
    const command = nodes.find(
      (n) => (n.data as { entity?: { kind?: string; id?: { slug?: string } } }).entity?.id?.slug === 'place-order',
    );
    expect(command).toBeDefined();

    const emitted = edges
      .filter((e) => e.source === command?.id && (e.data as { relation?: string })?.relation === 'emits')
      .map((e) => {
        const target = nodes.find((n) => n.id === e.target);
        return (target?.data as { entity?: { id?: { slug?: string } } }).entity?.id?.slug;
      })
      .sort();

    expect(emitted).toEqual(['order.placed', 'payment.requested']);
  });

  it('grows lane height to contain stacked events in the same moment', () => {
    const model = buildModel(stackedEventEntities() as never);
    const { nodes } = layoutBoard(model);
    const eventNodes = nodes
      .filter((n) => (n.data as { entity?: { kind?: string } }).entity?.kind === 'event')
      .sort((a, b) => a.position.y - b.position.y);
    expect(eventNodes).toHaveLength(2);
    expect(Math.round(eventNodes[0].position.x)).toBe(Math.round(eventNodes[1].position.x));
    expect(eventNodes[1].position.y).toBeGreaterThan(eventNodes[0].position.y);

    const lane = nodes.find((n) => n.id === 'lane:reserved:stream:unassigned');
    expect(lane).toBeDefined();
    const eventBottom = Math.max(...eventNodes.map((n) => n.position.y + NODE_H));
    expect(Number(lane?.height)).toBeGreaterThanOrEqual(eventBottom - (lane?.position.y ?? 0));
  });

  it('routes a UI no persona owns to the ui:unassigned catch-all lane', () => {
    const model = buildModel(stackedEventEntities() as never);
    const { nodes } = layoutBoard(model);
    const lane = nodes.find((n) => n.id === 'lane:reserved:ui:unassigned');
    expect(lane).toBeDefined();
    expect((lane?.data as { label?: string }).label).toBe('UI');

    const uiNode = nodes.find((n) => (n.data as { entity?: { kind?: string } }).entity?.kind === 'ui');
    expect(uiNode).toBeDefined();
    const top = lane?.position.y ?? 0;
    expect(uiNode?.position.y).toBeGreaterThanOrEqual(top);
    expect(uiNode?.position.y ?? 0).toBeLessThan(top + Number(lane?.height));
  });

  it('keeps a persona or swimlane slugged __unassigned__ out of the catch-all lanes', () => {
    const model = buildModel(reservedSlugEntities() as never);
    const { nodes } = layoutBoard(model);

    const laneIds = nodes.filter((n) => n.type === 'lane').map((n) => n.id);
    expect(new Set(laneIds).size).toBe(laneIds.length);
    for (const wanted of [
      'lane:entity:persona:shop/__unassigned__',
      'lane:reserved:ui:unassigned',
      'lane:entity:swimlane:shop/__unassigned__',
      'lane:reserved:stream:unassigned',
    ]) {
      expect(laneIds).toContain(wanted);
    }

    const laneIdAt = (y: number) =>
      nodes.find((n) => n.type === 'lane' && y >= n.position.y && y < n.position.y + Number(n.height))?.id;
    const laneIdOf = (slug: string) => {
      const sticky = nodes.find(
        (n) => n.type === 'sticky' && (n.data as { entity?: { id?: { slug?: string } } }).entity?.id?.slug === slug,
      );
      expect(sticky).toBeDefined();
      return laneIdAt(sticky?.position.y ?? -1);
    };

    expect(laneIdOf('owned')).toBe('lane:entity:persona:shop/__unassigned__');
    expect(laneIdOf('orphan')).toBe('lane:reserved:ui:unassigned');
    expect(laneIdOf('on-lane')).toBe('lane:entity:swimlane:shop/__unassigned__');
    expect(laneIdOf('off-lane')).toBe('lane:reserved:stream:unassigned');
  });

  it('creates no ui:unassigned lane when a command slice attributes every UI', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes } = layoutBoard(model);
    expect(nodes.find((n) => n.id === 'lane:reserved:ui:unassigned')).toBeUndefined();
    expect(nodes.find((n) => n.id === 'lane:entity:persona:shop/buyer')).toBeDefined();
  });

  it('attributes a display-only UI via its own UI slice persona', () => {
    const model = buildModel(displayOnlyUiOwnedByUiSliceEntities() as never);
    const { nodes } = layoutBoard(model);
    expect(nodes.find((n) => n.id === 'lane:reserved:ui:unassigned')).toBeUndefined();

    const personaLane = nodes.find((n) => n.id === 'lane:entity:persona:ops/manager');
    expect(personaLane).toBeDefined();
    const uiNode = nodes.find((n) => (n.data as { entity?: { kind?: string } }).entity?.kind === 'ui');
    expect(uiNode).toBeDefined();
    const top = personaLane?.position.y ?? 0;
    expect(uiNode?.position.y).toBeGreaterThanOrEqual(top);
    expect(uiNode?.position.y ?? 0).toBeLessThan(top + Number(personaLane?.height));
  });

  it('draws a UI slice as a read model rendering into its screen, left to right', () => {
    const model = buildModel(displayOnlyUiOwnedByUiSliceEntities() as never);
    const { nodes, edges } = layoutBoard(model);
    const sticky = (slug: string) =>
      nodes.find(
        (n) => n.type === 'sticky' && (n.data as { entity: { id: { slug: string } } }).entity.id.slug === slug,
      );
    const rm = sticky('orders-list');
    const ui = sticky('ops-console');
    expect(rm).toBeDefined();
    expect(ui).toBeDefined();
    // The screen never re-draws a read model the projection already placed:
    // the arrow reaches back to that one occurrence.
    expect(
      nodes.filter(
        (n) => n.type === 'sticky' && (n.data as { entity: { id: { slug: string } } }).entity.id.slug === 'orders-list',
      ),
    ).toHaveLength(1);
    expect(ui?.position.x ?? 0).toBeGreaterThan(rm?.position.x ?? 0);
    const rendersInto = edges.filter((e) => (e.data as { relation?: string } | undefined)?.relation === 'renders into');
    expect(rendersInto).toHaveLength(1);
    expect(rendersInto[0].source).toBe(rm?.id);
    expect(rendersInto[0].target).toBe(ui?.id);
  });

  it('attributes a display-only UI via the storyboard entry observer', () => {
    const model = buildModel(displayOnlyUiWatchedByStoryboardEntities() as never);
    const { nodes } = layoutBoard(model);
    expect(nodes.find((n) => n.id === 'lane:reserved:ui:unassigned')).toBeUndefined();

    const personaLane = nodes.find((n) => n.id === 'lane:entity:persona:shop/manager');
    expect(personaLane).toBeDefined();
    const uiNode = nodes.find((n) => (n.data as { entity?: { kind?: string } }).entity?.kind === 'ui');
    expect(uiNode).toBeDefined();
    const top = personaLane?.position.y ?? 0;
    expect(uiNode?.position.y).toBeGreaterThanOrEqual(top);
    expect(uiNode?.position.y ?? 0).toBeLessThan(top + Number(personaLane?.height));
  });

  it('pads a lane equally above and below its cards', () => {
    const model = buildModel(stackedEventEntities() as never);
    const { nodes } = layoutBoard(model);
    const laneNodes = nodes.filter((n) => n.type === 'lane');
    expect(laneNodes.length).toBeGreaterThan(0);
    for (const lane of laneNodes) {
      const cards = nodes.filter(
        (n) =>
          n.type === 'sticky' &&
          n.position.y >= lane.position.y &&
          n.position.y < lane.position.y + Number(lane.height),
      );
      if (cards.length === 0) continue;
      const top = Math.min(...cards.map((n) => n.position.y)) - lane.position.y;
      const bottom = lane.position.y + Number(lane.height) - Math.max(...cards.map((n) => n.position.y + NODE_H));
      expect(bottom).toBe(top);
    }
  });

  it('reserves no empty base column for an automation slice with reused sources', () => {
    const model = buildModel(reusedSourceAutomationEntities() as never);
    const { nodes } = layoutBoard(model);
    const sticky = (slug: string) =>
      nodes.find((n) => (n.data as { entity?: { id?: { slug?: string } } }).entity?.id?.slug === slug);
    const processor = sticky('fulfiller');
    const header = nodes.find(
      (n) =>
        n.id === 'slice:s2-fulfill-auto' ||
        (n.id.startsWith('slice:') &&
          (n.data as { entity?: { id?: { slug?: string } } }).entity?.id?.slug === 's2-fulfill-auto'),
    );
    expect(processor).toBeDefined();
    expect(header).toBeDefined();
    // The processor is the slice's leftmost occupant: its column starts
    // where the slice header starts, with no empty slot to its left.
    const headerX = header?.position.x ?? 0;
    const processorX = processor?.position.x ?? -1;
    expect(processorX).toBeGreaterThanOrEqual(headerX);
    expect(processorX - headerX, 'empty base column before the processor').toBeLessThan(264);
    // And the header spans exactly one column.
    expect(header?.width, 'slice header spans more than its content').toBeLessThanOrEqual(264);
  });

  it('fans an automation slice source read models side-by-side instead of stacking them', () => {
    const model = buildModel(multiSourceAutomationEntities() as never);
    const { nodes, edges } = layoutBoard(model);
    const rmNodes = nodes.filter((n) => (n.data as { entity?: { kind?: string } }).entity?.kind === 'readModel');
    expect(rmNodes).toHaveLength(3);
    const distinctX = new Set(rmNodes.map((n) => Math.round(n.position.x)));
    expect(distinctX.size, 'source read models stacked in one column').toBe(3);
    const distinctY = new Set(rmNodes.map((n) => Math.round(n.position.y)));
    expect(distinctY.size, 'source read models drifted off the timeline lane').toBe(1);

    const observed = edges.filter((e) => (e.data as { relation?: string }).relation === 'observed by');
    expect(observed).toHaveLength(3);
  });

  it('emits a cross-namespace edge (feeds context) when a read model references an upstream event', () => {
    const model = buildModel(crossNamespaceSeamEntities() as never);
    const { edges } = layoutBoard(model);
    const crossEdges = edges.filter((e) => (e.data as { relation?: string }).relation === 'feeds context');
    expect(crossEdges.length).toBeGreaterThan(0);
    for (const e of crossEdges) {
      const data = e.data as { source?: { id?: { namespace?: string } }; target?: { id?: { namespace?: string } } };
      expect(data.source?.id?.namespace).not.toBe(data.target?.id?.namespace);
    }
  });

  it('does not crash or duplicate nodes when an entity appears in more than one storyboard', () => {
    const model = buildModel(entityInMultipleStoryboardsEntities() as never);
    expect(() => layoutBoard(model)).not.toThrow();
    const { nodes } = layoutBoard(model);
    const commandNodes = nodes.filter((n) => (n.data as { entity?: { kind?: string } }).entity?.kind === 'command');
    const ids = commandNodes.map((n) => n.id);
    const uniqueIds = new Set(ids);
    expect(uniqueIds.size).toBe(ids.length);
    expect(commandNodes).toHaveLength(1);
  });

  it('does not crash on an empty storyboard that declares an entry observer', () => {
    const model = buildModel(emptyStoryboardWithEntryObserverEntities() as never);
    expect(() => layoutBoard(model)).not.toThrow();
    const { nodes } = layoutBoard(model);
    expect(nodes.length).toBeGreaterThan(0);
    const personaNode = nodes.find((n) => (n.data as { entity?: { kind?: string } }).entity?.kind === 'persona');
    expect(personaNode).toBeDefined();
  });

  it('keeps every automation source read model strictly left of the processor (11 sources)', () => {
    const ns = 'shop';
    const sources = Array.from({ length: 11 }, (_, i) => ({
      readModel: { id: id(ns, `rm-${i}`), title: `RM ${i}` },
    }));
    const model = buildModel([
      ...sources,
      { processor: { id: id(ns, 'proc'), title: 'Proc' } },
      { command: { id: id(ns, 'cmd'), title: 'Cmd' } },
      {
        automationSlice: {
          id: id(ns, 's-auto'),
          title: 'auto',
          processor: { processor: { id: id(ns, 'proc') } },
          emittedCommand: { command: { id: id(ns, 'cmd') } },
          sourceReadModels: sources.map((_, i) => ({ readModel: { id: id(ns, `rm-${i}`) } })),
        },
      },
      {
        storyboard: {
          id: id(ns, 'sb-auto'),
          title: 'auto',
          slices: [{ id: id(ns, 's-auto') }],
        },
      },
    ] as never);
    const { nodes } = layoutBoard(model);
    const proc = nodes.find((n) => (n.data as { entity?: { id?: { slug?: string } } }).entity?.id?.slug === 'proc');
    expect(proc).toBeDefined();
    const rms = nodes.filter((n) => (n.data as { entity?: { kind?: string } }).entity?.kind === 'readModel');
    expect(rms).toHaveLength(11);
    for (const rm of rms) {
      expect(
        rm.position.x,
        `${(rm.data as { entity: { id: { slug: string } } }).entity.id.slug} must stay left of processor`,
      ).toBeLessThan(proc!.position.x);
    }
  });
});

// A lane id becomes a ReactFlow node id, so two lanes sharing one is a
// rendering bug, not a cosmetic one. Bare ids (`stream:<slug>` alongside a
// literal `stream:unassigned`) collided three ways; each test below is one of
// them, and each failed before lane ids were split into the `entity:` and
// `reserved:` families.
describe('lane id families', () => {
  const laneIdsOf = (entities: Record<string, unknown>[]): string[] =>
    layoutBoard(buildModel(entities as never))
      .nodes.filter((n) => n.type === 'lane')
      .map((n) => n.id);

  it('keeps a swimlane slugged `unassigned` distinct from the stream catch-all', () => {
    const ids = laneIdsOf([
      { swimlane: { id: id('shop', 'unassigned'), title: 'Genuinely named unassigned' } },
      { event: { id: id('shop', 'on-lane'), title: 'On lane', swimlane: { id: id('shop', 'unassigned') } } },
      { event: { id: id('shop', 'off-lane'), title: 'Off lane' } },
    ]);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toContain('lane:entity:swimlane:shop/unassigned');
    expect(ids).toContain('lane:reserved:stream:unassigned');
  });

  it('gives the same slug in two namespaces a lane each', () => {
    const ids = laneIdsOf([
      { persona: { id: id('a', 'buyer'), title: 'A buyer' } },
      { persona: { id: id('b', 'buyer'), title: 'B buyer' } },
      { swimlane: { id: id('a', 'order'), title: 'A order' } },
      { swimlane: { id: id('b', 'order'), title: 'B order' } },
    ]);
    expect(new Set(ids).size).toBe(ids.length);
    for (const wanted of [
      'lane:entity:persona:a/buyer',
      'lane:entity:persona:b/buyer',
      'lane:entity:swimlane:a/order',
      'lane:entity:swimlane:b/order',
    ]) {
      expect(ids).toContain(wanted);
    }
  });

  it('draws one lane for a swimlane the store holds two versions of', () => {
    const entities = [
      { swimlane: { id: id('shop', 'order', '1'), title: 'Order v1' } },
      { swimlane: { id: id('shop', 'order', '2'), title: 'Order v2' } },
    ];
    const ids = laneIdsOf(entities);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids.filter((laneId) => laneId === 'lane:entity:swimlane:shop/order')).toHaveLength(1);

    const lane = layoutBoard(buildModel(entities as never)).nodes.find(
      (n) => n.id === 'lane:entity:swimlane:shop/order',
    );
    expect((lane?.data as { label?: string }).label).toBe('Order v2');
  });
});

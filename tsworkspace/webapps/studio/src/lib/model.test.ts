// Spot-check `buildModel`'s wire → display-model normalization. The goal
// is not exhaustive coverage of the assembler (the layout tests already
// exercise the derived shapes); it's to catch silent regressions in the
// per-kind envelope mapping, the version coercion, and the entity-key
// shape that every other module depends on.

import { describe, expect, it } from 'vitest';
import type { WireEntityEnvelope } from './api';
import { asId, buildModel, neighborsOf, neighborsOfMoment, staleRefsOf, versionOf } from './model';

function mustFind<T>(value: T | undefined): T {
  if (value === undefined) throw new Error('expected value to be present');
  return value;
}

describe('buildModel', () => {
  it('normalizes the canonical entity kinds from wire JSON', () => {
    const wire: WireEntityEnvelope[] = [
      {
        event: {
          id: { namespace: 'orders', slug: 'placed', version: '1' },
          title: 'Order placed',
        },
      },
      {
        command: {
          id: { namespace: 'orders', slug: 'place', version: '1' },
          title: 'Place order',
        },
      },
      {
        readModel: {
          id: { namespace: 'orders', slug: 'list', version: '1' },
          title: 'Order list',
        },
      },
      {
        ui: {
          id: { namespace: 'orders', slug: 'cart', version: '1' },
          title: 'Cart UI',
          slot: {
            screen: { id: { namespace: 'app', slug: 'checkout', version: '1' } },
            slot: 'main',
          },
        },
      },
      {
        schema: {
          id: { namespace: 'orders', slug: 'order-schema', version: '1' },
          title: 'Order schema',
        },
      },
      {
        project: {
          id: { namespace: 'shop', slug: 'shop', version: '1' },
          title: 'Shop',
        },
      },
      {
        screen: {
          id: { namespace: 'app', slug: 'checkout', version: '1' },
          title: 'Checkout',
        },
      },
      {
        term: {
          id: { namespace: 'orders', slug: 'review', version: '1' },
          title: 'review',
        },
      },
      {
        ambiguity: {
          id: { namespace: 'marketplace', slug: 'review-collision', version: '1' },
          doc: 'review means different things',
        },
      },
    ];
    const model = buildModel(wire);
    expect(model.entities.map((e) => e.kind).sort()).toEqual([
      'ambiguity',
      'command',
      'event',
      'project',
      'readModel',
      'schema',
      'screen',
      'term',
      'ui',
    ]);
    const event = model.entities.find((e) => e.kind === 'event');
    expect(event?.title).toBe('Order placed');
    expect(event?.id).toMatchObject({ namespace: 'orders', slug: 'placed', version: '1' });
    expect(event?.key).toContain('placed');
    const ui = model.entities.find((e) => e.kind === 'ui');
    expect(ui?.screenSlot).toMatchObject({ screen: { namespace: 'app', slug: 'checkout' }, slot: 'main' });
  });

  it('groups by namespace and ignores envelopes without a kind oneof', () => {
    const wire: WireEntityEnvelope[] = [
      { event: { id: { namespace: 'a', slug: 'x', version: '1' }, title: 'X' } },
      { event: { id: { namespace: 'b', slug: 'y', version: '1' }, title: 'Y' } },
      { unknownKind: { id: { namespace: 'c', slug: 'z', version: '1' } } } as WireEntityEnvelope,
      {} as WireEntityEnvelope,
    ];
    const model = buildModel(wire);
    expect(new Set(model.namespaces)).toEqual(new Set(['a', 'b']));
    expect(model.entities).toHaveLength(2);
  });
});

const mkId = (ns: string, slug: string) => ({ namespace: ns, slug, version: '1' });

function commandSliceFixture() {
  const ns = 'orders';
  return [
    { ui: { id: mkId(ns, 'cart'), title: 'Cart' } },
    { command: { id: mkId(ns, 'place'), title: 'Place order' } },
    { event: { id: mkId(ns, 'placed'), title: 'Order placed' } },
    { event: { id: mkId(ns, 'rejected'), title: 'Order rejected' } },
    {
      commandSlice: {
        id: mkId(ns, 's-place'),
        title: 'place order',
        ui: { ui: { id: mkId(ns, 'cart') } },
        command: { command: { id: mkId(ns, 'place') } },
        emittedEvents: [{ event: { id: mkId(ns, 'placed') } }, { event: { id: mkId(ns, 'rejected') } }],
      },
    },
  ] as WireEntityEnvelope[];
}

describe('neighborsOf', () => {
  it('command slice: ui forward to command, command backward to ui, command forward to events', () => {
    const model = buildModel(commandSliceFixture());
    const ui = mustFind(model.entities.find((e) => e.kind === 'ui'));
    const cmd = mustFind(model.entities.find((e) => e.kind === 'command'));
    const evPlaced = mustFind(model.entities.find((e) => e.id.slug === 'placed'));
    const evRejected = mustFind(model.entities.find((e) => e.id.slug === 'rejected'));

    const uiNeighbors = neighborsOf(model, ui);
    expect(uiNeighbors.forward.map((n) => n.entity.key)).toContain(cmd.key);
    expect(uiNeighbors.forward.map((n) => n.relation)).toContain('issues');
    expect(uiNeighbors.backward).toHaveLength(0);

    const cmdNeighbors = neighborsOf(model, cmd);
    expect(cmdNeighbors.backward.map((n) => n.entity.key)).toContain(ui.key);
    expect(cmdNeighbors.forward.map((n) => n.entity.key)).toContain(evPlaced.key);
    expect(cmdNeighbors.forward.map((n) => n.entity.key)).toContain(evRejected.key);

    const evNeighbors = neighborsOf(model, evPlaced);
    expect(evNeighbors.backward.map((n) => n.entity.key)).toContain(cmd.key);
    expect(evNeighbors.backward.map((n) => n.relation)).toContain('emitted by');
  });

  it('read model slice with cross-namespace source event adds projection neighbors', () => {
    const wire: WireEntityEnvelope[] = [
      { event: { id: mkId('upstream', 'thing.done'), title: 'thing.done' } },
      { readModel: { id: mkId('downstream', 'things'), title: 'things' } },
      { ui: { id: mkId('downstream', 'list-ui'), title: 'list UI' } },
      {
        readModelSlice: {
          id: mkId('downstream', 's-rm'),
          title: 'things slice',
          readModel: { readModel: { id: mkId('downstream', 'things') } },
          sourceEvents: [{ event: { id: mkId('upstream', 'thing.done') } }],
        },
      },
      {
        uiSlice: {
          id: mkId('downstream', 's-ui'),
          title: 'things list',
          sourceReadModels: [{ readModel: { id: mkId('downstream', 'things') } }],
          ui: { ui: { id: mkId('downstream', 'list-ui') } },
        },
      },
    ];
    const model = buildModel(wire);
    const ev = mustFind(model.entities.find((e) => e.kind === 'event'));
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    const ui = mustFind(model.entities.find((e) => e.kind === 'ui'));

    const evNeighbors = neighborsOf(model, ev);
    const crossRelation = evNeighbors.forward.find((n) => n.entity.key === rm.key)?.relation ?? '';
    expect(crossRelation).toContain('feeds context');

    const rmNeighbors = neighborsOf(model, rm);
    expect(rmNeighbors.backward.map((n) => n.entity.key)).toContain(ev.key);
    expect(rmNeighbors.forward.map((n) => n.entity.key)).toContain(ui.key);
  });

  it('automation slice: processor backward to read models, forward to command', () => {
    const wire: WireEntityEnvelope[] = [
      { readModel: { id: mkId('shop', 'queue'), title: 'queue' } },
      { processor: { id: mkId('shop', 'fulfiller'), title: 'fulfiller' } },
      { command: { id: mkId('shop', 'fulfill'), title: 'fulfill' } },
      {
        automationSlice: {
          id: mkId('shop', 's-auto'),
          title: 'auto fulfill',
          processor: { processor: { id: mkId('shop', 'fulfiller') } },
          emittedCommand: { command: { id: mkId('shop', 'fulfill') } },
          sourceReadModels: [{ readModel: { id: mkId('shop', 'queue') } }],
        },
      },
    ];
    const model = buildModel(wire);
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    const proc = mustFind(model.entities.find((e) => e.kind === 'processor'));
    const cmd = mustFind(model.entities.find((e) => e.kind === 'command'));

    const procNeighbors = neighborsOf(model, proc);
    expect(procNeighbors.backward.map((n) => n.entity.key)).toContain(rm.key);
    expect(procNeighbors.backward.map((n) => n.relation)).toContain('observes');
    expect(procNeighbors.forward.map((n) => n.entity.key)).toContain(cmd.key);
    expect(procNeighbors.forward.map((n) => n.relation)).toContain('issues');

    const rmNeighbors = neighborsOf(model, rm);
    expect(rmNeighbors.forward.map((n) => n.entity.key)).toContain(proc.key);
    expect(rmNeighbors.forward.map((n) => n.relation)).toContain('observed by');
  });

  it('swimlane lifecycle transitions add forward/backward neighbors for events', () => {
    const wire: WireEntityEnvelope[] = [
      { event: { id: mkId('shop', 'order.placed'), title: 'order.placed' } },
      { event: { id: mkId('shop', 'order.fulfilled'), title: 'order.fulfilled' } },
      {
        swimlane: {
          id: mkId('shop', 'order-lifecycle'),
          title: 'Order lifecycle',
          transitions: [
            {
              after: { id: mkId('shop', 'order.placed') },
              next: [{ event: { id: mkId('shop', 'order.fulfilled') } }],
            },
          ],
        },
      },
    ];
    const model = buildModel(wire);
    const placed = mustFind(model.entities.find((e) => e.id.slug === 'order.placed'));
    const fulfilled = mustFind(model.entities.find((e) => e.id.slug === 'order.fulfilled'));

    const placedNeighbors = neighborsOf(model, placed);
    expect(placedNeighbors.forward.map((n) => n.entity.key)).toContain(fulfilled.key);
    expect(placedNeighbors.forward.map((n) => n.relation)).toContain('lifecycle: then');

    const fulfilledNeighbors = neighborsOf(model, fulfilled);
    expect(fulfilledNeighbors.backward.map((n) => n.entity.key)).toContain(placed.key);
    expect(fulfilledNeighbors.backward.map((n) => n.relation)).toContain('lifecycle: after');
  });
});

describe('neighborsOfMoment', () => {
  it('read model moment: only shows the automation that observed the latest instance before it', () => {
    const wire: WireEntityEnvelope[] = [
      { readModel: { id: mkId('shop', 'queue'), title: 'queue' } },
      { processor: { id: mkId('shop', 'fulfiller'), title: 'fulfiller' } },
      { command: { id: mkId('shop', 'fulfill'), title: 'fulfill' } },
      { event: { id: mkId('shop', 'item.added'), title: 'item.added' } },
      {
        storyboard: {
          id: mkId('shop', 'sb1'),
          title: 'sb1',
          slices: [{ id: mkId('shop', 's-rm') }, { id: mkId('shop', 's-auto') }],
        },
      },
      {
        readModelSlice: {
          id: mkId('shop', 's-rm'),
          title: 'queue rm slice',
          readModel: { readModel: { id: mkId('shop', 'queue') } },
          sourceEvents: [{ event: { id: mkId('shop', 'item.added') } }],
        },
      },
      {
        automationSlice: {
          id: mkId('shop', 's-auto'),
          title: 'auto fulfill',
          processor: { processor: { id: mkId('shop', 'fulfiller') } },
          emittedCommand: { command: { id: mkId('shop', 'fulfill') } },
          sourceReadModels: [{ readModel: { id: mkId('shop', 'queue') } }],
        },
      },
    ];
    const model = buildModel(wire);
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    const proc = mustFind(model.entities.find((e) => e.kind === 'processor'));
    const rmSliceKey = mustFind(model.slices.find((s) => s.kind === 'readModelSlice')).entity.key;

    const moment = neighborsOfMoment(model, rm, rmSliceKey);
    expect(moment.forward.map((n) => n.entity.key)).toContain(proc.key);
    expect(moment.forward.map((n) => n.relation)).toContain('observed by');
  });

  it('falls back to neighborsOf when the moment key does not match any slice', () => {
    const model = buildModel(commandSliceFixture());
    const cmd = mustFind(model.entities.find((e) => e.kind === 'command'));
    const type = neighborsOf(model, cmd);
    const moment = neighborsOfMoment(model, cmd, 'nonexistent-moment-key');
    expect(moment.backward.map((n) => n.entity.key)).toEqual(type.backward.map((n) => n.entity.key));
    expect(moment.forward.map((n) => n.entity.key)).toEqual(type.forward.map((n) => n.entity.key));
  });
});

describe('versionOf', () => {
  it('returns 0 for null / undefined', () => {
    expect(versionOf(undefined)).toBe(0);
    expect(versionOf(null)).toBe(0);
  });

  it('parses wire string version into a number', () => {
    expect(versionOf({ version: '5' })).toBe(5);
    expect(versionOf({ version: '0' })).toBe(0);
  });

  it('returns 0 for unparseable input rather than NaN', () => {
    expect(versionOf({ version: 'not-a-number' })).toBe(0);
  });
});

describe('staleRefsOf', () => {
  // Migration-pending detection must compare pinned refs against the
  // highest loaded version of the subject, not whichever version happens
  // to appear first in `model.entities` (layout.ts uses max-version via
  // buildLatestBySlug; staleRefsOf must agree).
  it('flags a slice ref when a newer version exists later in the entity list', () => {
    const model = buildModel([
      {
        command: {
          id: { namespace: 'orders', slug: 'place', version: '1' },
          title: 'Place v1',
        },
      },
      {
        command: {
          id: { namespace: 'orders', slug: 'place', version: '2' },
          title: 'Place v2',
        },
      },
      {
        event: {
          id: { namespace: 'orders', slug: 'placed', version: '1' },
          title: 'Placed',
        },
      },
      {
        commandSlice: {
          id: { namespace: 'orders', slug: 'place-order', version: '1' },
          title: 'Place order',
          command: { command: { id: { namespace: 'orders', slug: 'place', version: '1' } } },
          emittedEvents: [{ event: { id: { namespace: 'orders', slug: 'placed', version: '1' } } }],
        },
      },
    ] as WireEntityEnvelope[]);
    const sliceEntity = mustFind(model.entities.find((e) => e.kind === 'commandSlice'));
    const stale = staleRefsOf(model, sliceEntity);
    expect(stale.map((s) => `${s.label}@${s.ref.version}->${s.latest}`)).toEqual(['command@1->2']);
  });
});

describe('asId with malformed raw values', () => {
  it('returns empty id for null', () => {
    expect(asId(null)).toEqual({ namespace: '', slug: '', version: '0' });
  });

  it('returns empty id for a string', () => {
    expect(asId('not-an-object')).toEqual({ namespace: '', slug: '', version: '0' });
  });

  it('returns empty id for a number', () => {
    expect(asId(42)).toEqual({ namespace: '', slug: '', version: '0' });
  });

  it('returns empty id for an array', () => {
    expect(asId(['a', 'b'])).toEqual({ namespace: '', slug: '', version: '0' });
  });

  it('returns well-formed id for a valid object', () => {
    expect(asId({ namespace: 'ns', slug: 'sl', version: '3' })).toEqual({
      namespace: 'ns',
      slug: 'sl',
      version: '3',
    });
  });
});

describe('normalizeEntity with malformed body', () => {
  it('drops an envelope where the kind body is a string', () => {
    const wire = [{ event: 'not-an-object' } as unknown as WireEntityEnvelope];
    const model = buildModel(wire);
    expect(model.entities).toHaveLength(0);
  });

  it('drops an envelope where the kind body is a number', () => {
    const wire = [{ event: 42 } as unknown as WireEntityEnvelope];
    const model = buildModel(wire);
    expect(model.entities).toHaveLength(0);
  });

  it('drops an envelope where the kind body is null', () => {
    const wire = [{ event: null } as unknown as WireEntityEnvelope];
    const model = buildModel(wire);
    expect(model.entities).toHaveLength(0);
  });

  it('handles calls field that is not an array without throwing', () => {
    const wire: WireEntityEnvelope[] = [
      {
        command: {
          id: { namespace: 'ns', slug: 'cmd', version: '1' },
          title: 'Cmd',
          calls: 'not-an-array' as unknown as never,
        },
      },
    ];
    const model = buildModel(wire);
    const cmd = model.entities.find((e) => e.kind === 'command');
    expect(cmd).toBeDefined();
    expect(cmd?.calls).toBeUndefined();
  });
});

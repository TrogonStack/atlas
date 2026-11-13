import { describe, expect, it } from 'vitest';
import { buildModel, type EntityId } from './model';
import { buildLifecycle, hasLifecycles, type Lifecycle, lifecycleLanes } from './statemachine';

const NS = 'acme-lifecycle';
const id = (slug: string, version = '1'): EntityId => ({ namespace: NS, slug, version });
const ref = (slug: string, version = '1') => ({ id: id(slug, version) });

type Wire = Parameters<typeof buildModel>[0];

function event(slug: string, lane = 'order', title = slug): Wire[number] {
  return { event: { id: id(slug), title, swimlane: ref(lane) } };
}

function lane(slug: string, rows: [string, (string | [string, string])[] | null][], version = '1'): Wire[number] {
  return {
    swimlane: {
      id: id(slug, version),
      title: slug,
      order: 1,
      transitions: rows.map(([after, next]) => ({
        after: ref(after),
        ...(next === null
          ? {}
          : { next: next.map((n) => (Array.isArray(n) ? { event: ref(n[0]), doc: n[1] } : { event: ref(n) })) }),
      })),
    },
  };
}

function build(rows: [string, (string | [string, string])[] | null][], extra: Wire = []): Lifecycle {
  const slugs = new Set(rows.flatMap(([a, n]) => [a, ...(n ?? []).map((x) => (Array.isArray(x) ? x[0] : x))]));
  const model = buildModel([lane('order', rows), ...[...slugs].map((s) => event(s)), ...extra]);
  const [l] = lifecycleLanes(model);
  return buildLifecycle(model, l);
}

const state = (lc: Lifecycle, slug: string) => {
  const s = lc.states.find((x) => x.ref.slug === slug);
  if (!s) throw new Error(`no state ${slug}`);
  return s;
};

describe('lifecycle layout', () => {
  it('tells a row, no row and an empty row apart', () => {
    const lc = build([
      ['placed', ['paid', 'cancelled']],
      ['paid', []],
    ]);
    expect(state(lc, 'placed').kind).toBe('row');
    expect(state(lc, 'paid').kind).toBe('final');
    expect(state(lc, 'cancelled').kind).toBe('open');
  });

  it('never draws an arrow back in time', () => {
    const lc = build([
      ['placed', ['reserved', 'expired', 'cancelled']],
      ['reserved', ['paid', 'expired']],
      ['expired', ['reserved', 'cancelled']],
      ['paid', ['refunded', 'shipped']],
      ['shipped', ['delivered', 'refunded']],
      ['refunded', ['paid']],
      ['delivered', []],
    ]);
    const column = new Map([...lc.states, ...lc.stubs].map((n) => [n.id, n.column]));
    for (const a of lc.arrows) {
      expect(column.get(a.to)).toBeGreaterThan(column.get(a.from) ?? Infinity);
      for (let i = 1; i < a.points.length; i++) expect(a.points[i].x).toBeGreaterThan(a.points[i - 1].x);
    }
    expect(lc.stubs.map((s) => [s.from, s.returnsTo]).sort()).toEqual([
      [`event:${NS}/expired`, `event:${NS}/reserved`],
      [`event:${NS}/refunded`, `event:${NS}/paid`],
    ]);
  });

  it('draws a self-loop as a stub back to the same state', () => {
    const lc = build([['retried', ['retried', 'done']]]);
    expect(lc.stubs).toHaveLength(1);
    expect(lc.stubs[0].returnsTo).toBe(lc.stubs[0].from);
    const loop = lc.arrows.find((a) => a.reentry);
    expect(loop?.target).toBe(`event:${NS}/retried`);
  });

  it('draws a lifecycle that is one cycle, starting from its first row', () => {
    const lc = build([
      ['open', ['closed']],
      ['closed', ['reopened']],
      ['reopened', ['closed']],
    ]);
    expect(state(lc, 'open').column).toBe(0);
    expect(lc.stubs.map((s) => s.title)).toEqual(['closed']);
  });

  it('draws a cycle with no entry point', () => {
    const lc = build([
      ['a', ['b']],
      ['b', ['c']],
      ['c', ['a']],
    ]);
    expect(lc.states.map((s) => [s.ref.slug, s.column])).toEqual([
      ['a', 0],
      ['b', 1],
      ['c', 2],
    ]);
    expect(lc.stubs).toHaveLength(1);
    expect(lc.stubs[0].returnsTo).toBe(`event:${NS}/a`);
  });

  it('keeps a missing or foreign event visible and marks it', () => {
    const model = buildModel([
      lane('order', [['placed', ['ghost', 'charged']]]),
      event('placed'),
      event('charged', 'payment'),
    ]);
    const lc = buildLifecycle(model, lifecycleLanes(model)[0]);
    expect(state(lc, 'ghost')).toMatchObject({ problem: 'missing', title: 'ghost' });
    expect(state(lc, 'charged').problem).toBe('foreign');
    expect(state(lc, 'placed').problem).toBeUndefined();
  });

  it('lists the stream events no transition names, and only those', () => {
    const lc = build([['placed', ['paid']]], [event('note-added', 'order', 'Note added'), event('charged', 'payment')]);
    expect(lc.untracked.map((e) => e.title)).toEqual(['Note added']);
  });

  it('keeps guards and their order in the row', () => {
    const lc = build([
      [
        'placed',
        [
          ['paid', 'Payment clears before the hold expires.'],
          ['expired', 'The hold expires first.'],
        ],
      ],
    ]);
    const out = lc.arrows.filter((a) => a.from === `event:${NS}/placed`).sort((a, b) => a.order - b.order);
    expect(out.map((a) => a.guard)).toEqual(['Payment clears before the hold expires.', 'The hold expires first.']);
  });

  it('routes an arrow that skips a column through a waypoint', () => {
    const lc = build([
      ['a', ['b', 'c']],
      ['b', ['c']],
    ]);
    const skip = lc.arrows.find((a) => a.from === `event:${NS}/a` && a.to === `event:${NS}/c`);
    expect(skip?.points).toHaveLength(3);
  });

  it('reads two rows for one event as one row', () => {
    const lc = build([
      ['placed', ['paid']],
      ['placed', ['cancelled', 'paid']],
    ]);
    expect(lc.arrows.filter((a) => a.from === `event:${NS}/placed`).map((a) => a.target)).toEqual([
      `event:${NS}/paid`,
      `event:${NS}/cancelled`,
    ]);
  });

  it('lays out the same table the same way every time', () => {
    const rows: [string, string[]][] = [
      ['placed', ['reserved', 'cancelled']],
      ['reserved', ['paid', 'placed']],
    ];
    expect(build(rows)).toEqual(build(rows));
  });
});

describe('streams with a lifecycle', () => {
  it('uses the newest version of a stream and skips streams with no transitions', () => {
    const model = buildModel([
      lane('order', [['placed', ['paid']]], '1'),
      lane('order', [['placed', ['paid', 'cancelled']]], '2'),
      { swimlane: { id: id('audit'), title: 'audit', order: 2 } },
      event('placed'),
      event('paid'),
      event('cancelled'),
    ]);
    const lanes = lifecycleLanes(model);
    expect(lanes.map((l) => [l.id.slug, l.id.version])).toEqual([['order', '2']]);
    expect(buildLifecycle(model, lanes[0]).arrows).toHaveLength(2);
  });

  it('reports no lifecycle when no stream declares transitions', () => {
    expect(hasLifecycles(buildModel([{ swimlane: { id: id('order'), title: 'order' } }, event('placed')]))).toBe(false);
  });
});

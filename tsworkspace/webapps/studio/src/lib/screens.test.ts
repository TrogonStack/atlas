// Tests for buildScreens in screens.ts.
// Covers: empty model, command slices with UI, read model slices with UI,
// automation slices as system steps, cross-storyboard rows, and edge
// structural invariants.

import { describe, expect, it } from 'vitest';
import { buildModel } from './model';
import { buildScreens, FRAME_H, FRAME_W, SYS_H, type SystemStepData } from './screens';

function mustFind<T>(value: T | undefined): T {
  if (value === undefined) throw new Error('expected value to be present');
  return value;
}

const id = (namespace: string, slug: string) => ({ namespace, slug, version: '1' });

describe('buildScreens: empty model', () => {
  it('returns zero nodes and zero edges for an empty model', () => {
    const model = buildModel([]);
    const { nodes, edges } = buildScreens(model);
    expect(nodes).toHaveLength(0);
    expect(edges).toHaveLength(0);
  });
});

describe('buildScreens: single commandSlice with UI', () => {
  function singleCommandModel() {
    return buildModel([
      { persona: { id: id('shop', 'buyer'), title: 'Buyer' } },
      { ui: { id: id('shop', 'home'), title: 'Home' } },
      { command: { id: id('shop', 'buy'), title: 'Buy' } },
      { event: { id: id('shop', 'bought'), title: 'Bought' } },
      {
        commandSlice: {
          id: id('shop', 's1'),
          title: 'buy slice',
          persona: { persona: { id: id('shop', 'buyer') } },
          ui: { ui: { id: id('shop', 'home') } },
          command: { command: { id: id('shop', 'buy') } },
          emittedEvents: [{ event: { id: id('shop', 'bought') } }],
        },
      },
    ] as never);
  }

  it('produces at least one screenFrame node', () => {
    const { nodes } = buildScreens(singleCommandModel());
    expect(nodes.some((n) => n.type === 'screenFrame')).toBe(true);
  });

  it('the screenFrame node carries the correct ui entity title', () => {
    const { nodes } = buildScreens(singleCommandModel());
    const frame = nodes.find((n) => n.type === 'screenFrame');
    expect(frame).toBeDefined();
    const data = mustFind(frame).data as { ui: { title: string } };
    expect(data.ui.title).toBe('Home');
  });

  it('the screenFrame actions list includes the Buy command', () => {
    const { nodes } = buildScreens(singleCommandModel());
    const frame = nodes.find((n) => n.type === 'screenFrame');
    const data = mustFind(frame).data as { actions: { title: string }[] };
    expect(data.actions.some((a) => a.title === 'Buy')).toBe(true);
  });
});

describe('buildScreens: uiSlice', () => {
  it('adds the read model to the frame displays list', () => {
    const model = buildModel([
      { ui: { id: id('shop', 'catalog'), title: 'Catalog' } },
      { readModel: { id: id('shop', 'items'), title: 'Items' } },
      { event: { id: id('shop', 'ev'), title: 'Ev' } },
      {
        readModelSlice: {
          id: id('shop', 's1'),
          title: 'catalog slice',
          readModel: { readModel: { id: id('shop', 'items') } },
          sourceEvents: [{ event: { id: id('shop', 'ev') } }],
        },
      },
      {
        uiSlice: {
          id: id('shop', 's2'),
          title: 'catalog displays items',
          sourceReadModels: [{ readModel: { id: id('shop', 'items') } }],
          ui: { ui: { id: id('shop', 'catalog') } },
        },
      },
    ] as never);
    const { nodes } = buildScreens(model);
    const frame = nodes.find((n) => n.type === 'screenFrame');
    expect(frame).toBeDefined();
    const data = mustFind(frame).data as { displays: { title: string }[] };
    expect(data.displays.some((d) => d.title === 'Items')).toBe(true);
  });
});

describe('buildScreens: a projection alone becomes a systemStep', () => {
  it('produces a systemStep node for a readModelSlice', () => {
    const model = buildModel([
      { readModel: { id: id('shop', 'feed'), title: 'Feed' } },
      { event: { id: id('shop', 'ev'), title: 'Ev' } },
      {
        readModelSlice: {
          id: id('shop', 's1'),
          title: 'feed slice',
          readModel: { readModel: { id: id('shop', 'feed') } },
          sourceEvents: [{ event: { id: id('shop', 'ev') } }],
        },
      },
    ] as never);
    const { nodes } = buildScreens(model);
    expect(nodes.some((n) => n.type === 'systemStep')).toBe(true);
  });
});

describe('buildScreens: automation slice produces a systemStep', () => {
  it('maps an automationSlice to a systemStep node', () => {
    const model = buildModel([
      { processor: { id: id('shop', 'notifier'), title: 'Notifier' } },
      { command: { id: id('shop', 'notify'), title: 'Notify' } },
      { readModel: { id: id('shop', 'rm'), title: 'RM' } },
      {
        automationSlice: {
          id: id('shop', 'as1'),
          title: 'notify automation',
          processor: { processor: { id: id('shop', 'notifier') } },
          command: { command: { id: id('shop', 'notify') } },
          sourceReadModels: [{ readModel: { id: id('shop', 'rm') } }],
        },
      },
    ] as never);
    const { nodes } = buildScreens(model);
    expect(nodes.some((n) => n.type === 'systemStep')).toBe(true);
  });

  it('names the automation glyph on the step line instead of prefixing the text', () => {
    const model = buildModel([
      { processor: { id: id('shop', 'notifier'), title: 'Notifier' } },
      {
        automationSlice: {
          id: id('shop', 'as1'),
          title: 'notify automation',
          processor: { processor: { id: id('shop', 'notifier') } },
        },
      },
    ] as never);
    const { nodes } = buildScreens(model);
    const step = nodes.find((n) => n.type === 'systemStep');
    expect((step?.data as SystemStepData).lines).toEqual([{ text: 'Notifier', glyph: 'automation' }]);
  });
});

describe('buildScreens: storyboard slicing', () => {
  it('separates slices into rows per storyboard', () => {
    const model = buildModel([
      { ui: { id: id('shop', 'ui-a'), title: 'UI A' } },
      { ui: { id: id('shop', 'ui-b'), title: 'UI B' } },
      { command: { id: id('shop', 'cmd-a'), title: 'Cmd A' } },
      { command: { id: id('shop', 'cmd-b'), title: 'Cmd B' } },
      { event: { id: id('shop', 'ev-a'), title: 'Ev A' } },
      { event: { id: id('shop', 'ev-b'), title: 'Ev B' } },
      {
        commandSlice: {
          id: id('shop', 'sa'),
          title: 'slice a',
          ui: { ui: { id: id('shop', 'ui-a') } },
          command: { command: { id: id('shop', 'cmd-a') } },
          emittedEvents: [{ event: { id: id('shop', 'ev-a') } }],
        },
      },
      {
        commandSlice: {
          id: id('shop', 'sb'),
          title: 'slice b',
          ui: { ui: { id: id('shop', 'ui-b') } },
          command: { command: { id: id('shop', 'cmd-b') } },
          emittedEvents: [{ event: { id: id('shop', 'ev-b') } }],
        },
      },
      {
        storyboard: {
          id: id('shop', 'sb1'),
          title: 'Flow A',
          slices: [{ id: id('shop', 'sa') }],
        },
      },
      {
        storyboard: {
          id: id('shop', 'sb2'),
          title: 'Flow B',
          slices: [{ id: id('shop', 'sb') }],
        },
      },
    ] as never);
    const { nodes } = buildScreens(model);
    const sbHeaders = nodes.filter((n) => n.type === 'seqStoryboardHeader');
    expect(sbHeaders).toHaveLength(2);
  });
});

describe('buildScreens: edge structural invariant', () => {
  it('only emits edges whose source and target nodes exist', () => {
    const model = buildModel([
      { ui: { id: id('shop', 'home'), title: 'Home' } },
      { command: { id: id('shop', 'go'), title: 'Go' } },
      { event: { id: id('shop', 'gone'), title: 'Gone' } },
      { ui: { id: id('shop', 'detail'), title: 'Detail' } },
      { readModel: { id: id('shop', 'rm'), title: 'RM' } },
      {
        commandSlice: {
          id: id('shop', 's1'),
          title: 'go slice',
          ui: { ui: { id: id('shop', 'home') } },
          command: { command: { id: id('shop', 'go') } },
          emittedEvents: [{ event: { id: id('shop', 'gone') } }],
        },
      },
      {
        readModelSlice: {
          id: id('shop', 's2'),
          title: 'rm slice',
          readModel: { readModel: { id: id('shop', 'rm') } },
          sourceEvents: [{ event: { id: id('shop', 'gone') } }],
        },
      },
      {
        uiSlice: {
          id: id('shop', 's3'),
          title: 'detail shows the rm',
          sourceReadModels: [{ readModel: { id: id('shop', 'rm') } }],
          ui: { ui: { id: id('shop', 'detail') } },
        },
      },
    ] as never);
    const { nodes, edges } = buildScreens(model);
    const ids = new Set(nodes.map((n) => n.id));
    for (const e of edges) {
      expect(ids.has(e.source), `dangling source ${e.source}`).toBe(true);
      expect(ids.has(e.target), `dangling target ${e.target}`).toBe(true);
    }
  });
});

describe('buildScreens: exported dimension constants', () => {
  it('FRAME_W and FRAME_H are positive numbers', () => {
    expect(FRAME_W).toBeGreaterThan(0);
    expect(FRAME_H).toBeGreaterThan(0);
  });

  it('SYS_H is a positive number smaller than FRAME_H', () => {
    expect(SYS_H).toBeGreaterThan(0);
    expect(SYS_H).toBeLessThan(FRAME_H);
  });
});

describe('buildScreens: latest version resolution', () => {
  // BUG: bySlug is a plain Map last-wins over entity array order, while
  // layoutBoard/plan pick max versionOf. An older version listed later
  // shadows the latest title/fields on the UI frame.
  it('resolves the latest UI version when multiple versions exist', () => {
    const model = buildModel([
      { ui: { id: { namespace: 'shop', slug: 'home', version: '2' }, title: 'Home v2' } },
      { ui: { id: { namespace: 'shop', slug: 'home', version: '1' }, title: 'Home v1' } },
      { command: { id: id('shop', 'buy'), title: 'Buy' } },
      { event: { id: id('shop', 'bought'), title: 'Bought' } },
      {
        commandSlice: {
          id: id('shop', 's1'),
          title: 'buy slice',
          ui: { ui: { id: { namespace: 'shop', slug: 'home', version: '2' } } },
          command: { command: { id: id('shop', 'buy') } },
          emittedEvents: [{ event: { id: id('shop', 'bought') } }],
        },
      },
    ] as never);
    const { nodes } = buildScreens(model);
    const frame = nodes.find((n) => n.type === 'screenFrame');
    expect(frame).toBeDefined();
    const data = mustFind(frame).data as { ui: { title: string; id: { version: string } } };
    expect(data.ui.title).toBe('Home v2');
    expect(data.ui.id.version).toBe('2');
  });
});

// Tests for buildPlan in plan.ts.
// Covers: empty model, single-phase models, multi-phase layering,
// edge deduplication, and node/edge structural invariants.

import { describe, expect, it } from 'vitest';
import { buildModel } from './model';
import { buildPlan, PLAN_NODE_W } from './plan';

function mustFind<T>(value: T | undefined): T {
  if (value === undefined) throw new Error('expected value to be present');
  return value;
}

const id = (namespace: string, slug: string) => ({ namespace, slug, version: '1' });

describe('buildPlan: empty model', () => {
  it('returns zero nodes, zero edges, and zero phases for an empty model', () => {
    const model = buildModel([]);
    const result = buildPlan(model);
    expect(result.nodes).toHaveLength(0);
    expect(result.edges).toHaveLength(0);
    expect(result.phases).toBe(0);
  });
});

describe('buildPlan: single commandSlice', () => {
  function singleCommandModel() {
    return buildModel([
      { event: { id: id('shop', 'ordered'), title: 'Ordered' } },
      { command: { id: id('shop', 'place-order'), title: 'Place Order' } },
      {
        commandSlice: {
          id: id('shop', 's1'),
          title: 'place order slice',
          command: { command: { id: id('shop', 'place-order') } },
          emittedEvents: [{ event: { id: id('shop', 'ordered') } }],
        },
      },
    ] as never);
  }

  it('produces at least one phase', () => {
    const { phases } = buildPlan(singleCommandModel());
    expect(phases).toBeGreaterThanOrEqual(1);
  });

  it('produces planArtifact nodes for the command and event', () => {
    const { nodes } = buildPlan(singleCommandModel());
    const artifactNodes = nodes.filter((n) => n.type === 'planArtifact');
    expect(artifactNodes.length).toBeGreaterThanOrEqual(2);
  });

  it('produces planPhase label nodes equal to the phase count', () => {
    const { nodes, phases } = buildPlan(singleCommandModel());
    const phaseNodes = nodes.filter((n) => n.type === 'planPhase');
    expect(phaseNodes).toHaveLength(phases);
  });

  it('event lands in an earlier phase than the command (command depends on event)', () => {
    const { nodes } = buildPlan(singleCommandModel());
    const eventNode = nodes.find((n) => n.type === 'planArtifact' && n.id.includes('shop/ordered'));
    const cmdNode = nodes.find((n) => n.type === 'planArtifact' && n.id.includes('shop/place-order'));
    expect(eventNode).toBeDefined();
    expect(cmdNode).toBeDefined();
    expect(mustFind(eventNode).position.y).toBeLessThan(mustFind(cmdNode).position.y);
  });

  it('all edges reference nodes that exist in the node set', () => {
    const { nodes, edges } = buildPlan(singleCommandModel());
    const ids = new Set(nodes.map((n) => n.id));
    for (const e of edges) {
      expect(ids.has(e.source), `dangling source ${e.source}`).toBe(true);
      expect(ids.has(e.target), `dangling target ${e.target}`).toBe(true);
    }
  });
});

describe('buildPlan: multi-phase projection-then-display chain', () => {
  function chainModel() {
    // event → readModel (phase 1) → ui (phase 2)
    return buildModel([
      { event: { id: id('shop', 'ev'), title: 'Ev' } },
      { readModel: { id: id('shop', 'rm'), title: 'RM' } },
      { ui: { id: id('shop', 'home'), title: 'Home' } },
      {
        readModelSlice: {
          id: id('shop', 's1'),
          title: 'rm slice',
          readModel: { readModel: { id: id('shop', 'rm') } },
          sourceEvents: [{ event: { id: id('shop', 'ev') } }],
        },
      },
      {
        uiSlice: {
          id: id('shop', 's2'),
          title: 'home shows rm',
          sourceReadModels: [{ readModel: { id: id('shop', 'rm') } }],
          ui: { ui: { id: id('shop', 'home') } },
        },
      },
    ] as never);
  }

  it('places at least 3 phases for the three-node chain', () => {
    const { phases } = buildPlan(chainModel());
    expect(phases).toBeGreaterThanOrEqual(3);
  });

  it('PLAN_NODE_W export is a positive number', () => {
    expect(PLAN_NODE_W).toBeGreaterThan(0);
  });
});

describe('buildPlan: edge deduplication', () => {
  it('does not emit duplicate edges for the same artifact pair', () => {
    // Two slices both referencing the same event→command link.
    const model = buildModel([
      { event: { id: id('shop', 'ev'), title: 'Ev' } },
      { command: { id: id('shop', 'cmd'), title: 'Cmd' } },
      {
        commandSlice: {
          id: id('shop', 's1'),
          title: 'slice 1',
          command: { command: { id: id('shop', 'cmd') } },
          emittedEvents: [{ event: { id: id('shop', 'ev') } }],
        },
      },
      {
        commandSlice: {
          id: id('shop', 's2'),
          title: 'slice 2',
          command: { command: { id: id('shop', 'cmd') } },
          emittedEvents: [{ event: { id: id('shop', 'ev') } }],
        },
      },
    ] as never);
    const { edges } = buildPlan(model);
    const edgeIds = edges.map((e) => e.id);
    const unique = new Set(edgeIds);
    expect(unique.size).toBe(edgeIds.length);
  });
});

describe('buildPlan: scenario counts roll up', () => {
  it('scenarioCount on a planArtifact node equals total scenarios across touching slices', () => {
    const model = buildModel([
      { event: { id: id('shop', 'ev'), title: 'Ev' } },
      { command: { id: id('shop', 'cmd'), title: 'Cmd' } },
      {
        commandSlice: {
          id: id('shop', 's1'),
          title: 'slice 1',
          command: { command: { id: id('shop', 'cmd') } },
          emittedEvents: [{ event: { id: id('shop', 'ev') } }],
          scenarios: [
            { id: 'sc1', title: 'Happy path' },
            { id: 'sc2', title: 'Sad path' },
          ],
        },
      },
    ] as never);
    const { nodes } = buildPlan(model);
    const cmdNode = nodes.find((n) => n.type === 'planArtifact' && n.id.includes('shop/cmd'));
    expect(cmdNode).toBeDefined();
    const data = mustFind(cmdNode).data as { scenarioCount: number };
    expect(data.scenarioCount).toBe(2);
  });
});

describe('buildPlan: UI without persona', () => {
  // BUG: commandSlice gates UI inclusion on `s.persona`, so a screen that
  // triggers a command but has no persona never appears as a deliverable,
  // unlike readModelSlice which includes UI unconditionally.
  it('includes the UI deliverable when a commandSlice declares a UI without a persona', () => {
    const model = buildModel([
      { event: { id: id('shop', 'ordered'), title: 'Ordered' } },
      { command: { id: id('shop', 'place-order'), title: 'Place Order' } },
      { ui: { id: id('shop', 'checkout'), title: 'Checkout' } },
      {
        commandSlice: {
          id: id('shop', 's1'),
          title: 'place order slice',
          ui: { ui: { id: id('shop', 'checkout') } },
          command: { command: { id: id('shop', 'place-order') } },
          emittedEvents: [{ event: { id: id('shop', 'ordered') } }],
        },
      },
    ] as never);
    const { nodes } = buildPlan(model);
    const uiNode = nodes.find((n) => n.type === 'planArtifact' && n.id.includes('shop/checkout'));
    expect(uiNode).toBeDefined();
  });
});

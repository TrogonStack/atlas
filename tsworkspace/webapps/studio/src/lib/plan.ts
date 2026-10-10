// Implementation-plan projection of the model, CONTRACT-FIRST and in pure
// execution terms: the nodes are the DELIVERABLES (events, commands, read
// models, screens, processors). Slices are not work items; they are the specs
// of these deliverables (a read model touched by three slices is one
// projection with three cases), so their scenario counts roll up onto the
// artifact they specify and the drawer lists the slices on click.
//
//   events                        pure contracts, no dependencies (Phase 1)
//   read model  ← source events   (the projection's inputs)
//   command     ← emitted events  (its contract is the events it produces)
//   screen      ← read models it renders, + commands it triggers
//   processor   ← read models it observes, + commands it issues
//
// The graph is acyclic by construction, phases are longest-path layers
// flowing DOWNWARD: everything in a row is independent, parallel work.

import type { Edge, Node } from '@xyflow/react';
import { type Entity, type EntityKind, type Model, orderedSlices, slugKey, trackingOf, versionOf } from './model';

export const PLAN_NODE_W = 232;
const ITEM_W = PLAN_NODE_W + 28; // horizontal pitch inside a phase row
const PHASE_H = 116; // vertical pitch between phases
const PHASE_LABEL_W = 176;

export interface PlanArtifactData extends Record<string, unknown> {
  entity: Entity;
  sliceCount: number;
  scenarioCount: number;
  // Delivery status from tracker overlays: the Plan is the execution view.
  status?: string;
}

export interface PlanPhaseData extends Record<string, unknown> {
  phase: number;
  items: number;
}

export function buildPlan(model: Model): { nodes: Node[]; edges: Edge[]; phases: number } {
  const slices = orderedSlices(model);

  // Latest entity per slug, to resolve slice EdgeRefs into artifact nodes.
  const bySlug = new Map<string, Entity>();
  for (const e of model.entities) {
    const k = slugKey(e.kind, e.id);
    const cur = bySlug.get(k);
    if (!cur || versionOf(e.id) > versionOf(cur.id)) bySlug.set(k, e);
  }

  const artifacts = new Map<string, Entity>();
  const firstTouch = new Map<string, number>();
  const sliceCount = new Map<string, number>();
  const scenarioCount = new Map<string, number>();
  const touch = (e: Entity | undefined, sliceIdx: number) => {
    if (!e) return;
    if (!firstTouch.has(e.key) || sliceIdx < (firstTouch.get(e.key) ?? 0)) firstTouch.set(e.key, sliceIdx);
  };
  const spec = (e: Entity | undefined, scenarios: number) => {
    if (!e) return;
    sliceCount.set(e.key, (sliceCount.get(e.key) ?? 0) + 1);
    scenarioCount.set(e.key, (scenarioCount.get(e.key) ?? 0) + scenarios);
  };
  const artifact = (kind: EntityKind, id?: { namespace: string; slug: string }): Entity | undefined => {
    if (!id) return undefined;
    const e = bySlug.get(slugKey(kind, id));
    if (!e) return undefined;
    if (!artifacts.has(e.key)) artifacts.set(e.key, e);
    return e;
  };

  const deps: { from: string; to: string }[] = [];
  const seenDeps = new Set<string>();
  const add = (from?: Entity, to?: Entity) => {
    if (!from || !to || from.key === to.key) return;
    const k = `${from.key}->${to.key}`;
    if (seenDeps.has(k)) return;
    seenDeps.add(k);
    deps.push({ from: from.key, to: to.key });
  };

  slices.forEach((s, i) => {
    if (s.kind === 'commandSlice') {
      const cmd = artifact('command', s.command?.id);
      const ui = s.ui ? artifact('ui', s.ui.id) : undefined;
      add(cmd, ui); // the screen is coded against the command's contract
      for (const e of s.events) add(artifact('event', e.id), cmd); // its contract includes what it emits
      spec(cmd, s.scenarios.length);
      touch(cmd, i);
      touch(ui, i);
      for (const e of s.events) touch(artifact('event', e.id), i);
    }
    if (s.kind === 'readModelSlice') {
      const rm = artifact('readModel', s.readModel?.id);
      for (const e of s.events) add(artifact('event', e.id), rm); // the projection's inputs
      spec(rm, s.scenarios.length);
      touch(rm, i);
      for (const e of s.events) touch(artifact('event', e.id), i);
    }
    if (s.kind === 'uiSlice') {
      const ui = artifact('ui', s.ui?.id);
      for (const r of s.readModels) add(artifact('readModel', r.id), ui); // the screen renders the views
      touch(ui, i);
      for (const r of s.readModels) touch(artifact('readModel', r.id), i);
    }
    if (s.kind === 'automationSlice') {
      const proc = artifact('processor', s.processor?.id);
      const cmd = artifact('command', s.command?.id);
      for (const r of s.readModels) add(artifact('readModel', r.id), proc); // what it observes
      add(cmd, proc); // the processor is coded against the command's contract
      spec(proc, s.scenarios.length);
      touch(proc, i);
      touch(cmd, i);
      for (const r of s.readModels) touch(artifact('readModel', r.id), i);
    }
  });

  const allKeys = [...artifacts.keys()];
  // Empty model → empty plan (no phantom phase). Without this guard
  // `Math.max(..., 0) + 1` would emit a `phases=1` row with zero items.
  if (allKeys.length === 0) {
    return { nodes: [], edges: [], phases: 0 };
  }
  const preds = new Map<string, string[]>(allKeys.map((k) => [k, []]));
  for (const d of deps) preds.get(d.to)?.push(d.from);

  // Longest-path layering: a deliverable lands one phase after the latest
  // contract it needs. Acyclic by construction.
  const layers = new Map<string, number>();
  const layerOf = (k: string): number => {
    const cached = layers.get(k);
    if (cached !== undefined) return cached;
    let max = -1;
    for (const p of preds.get(k) ?? []) max = Math.max(max, layerOf(p));
    const layer = max + 1;
    layers.set(k, layer);
    return layer;
  };
  for (const k of allKeys) layerOf(k);
  const phases = Math.max(...allKeys.map((k) => layers.get(k) ?? 0), 0) + 1;

  const byLayer = new Map<number, string[]>();
  for (const k of allKeys) {
    const l = layers.get(k) ?? 0;
    const list = byLayer.get(l) ?? [];
    list.push(k);
    byLayer.set(l, list);
  }

  // Balanced tree layout: every phase row is centered on the widest row, and
  // items sort by the barycenter of their dependencies' positions so each
  // node hangs under what it needs.
  const maxCount = Math.max(...[...byLayer.values()].map((l) => l.length), 1);
  const xCenter = new Map<string, number>();
  const placed = new Map<string, { x: number; y: number }>();
  for (let l = 0; l < phases; l++) {
    const list = byLayer.get(l) ?? [];
    const scored = list.map((k) => {
      const ps = (preds.get(k) ?? []).map((p) => xCenter.get(p)).filter((x): x is number => x !== undefined);
      const bary = ps.length > 0 ? ps.reduce((a, b) => a + b, 0) / ps.length : Number.NaN;
      return { k, bary };
    });
    scored.sort((a, b) => {
      const an = Number.isNaN(a.bary);
      const bn = Number.isNaN(b.bary);
      if (an && bn) return (firstTouch.get(a.k) ?? 0) - (firstTouch.get(b.k) ?? 0);
      if (an) return -1;
      if (bn) return 1;
      return a.bary - b.bary || (firstTouch.get(a.k) ?? 0) - (firstTouch.get(b.k) ?? 0);
    });
    const x0 = ((maxCount - list.length) * ITEM_W) / 2;
    scored.forEach(({ k }, idx) => {
      const x = x0 + idx * ITEM_W;
      xCenter.set(k, x + PLAN_NODE_W / 2);
      placed.set(k, { x, y: l * PHASE_H });
    });
  }

  const nodes: Node[] = [];
  for (let l = 0; l < phases; l++) {
    nodes.push({
      id: `phase:${l}`,
      type: 'planPhase',
      position: { x: -PHASE_LABEL_W - 28, y: l * PHASE_H },
      data: { phase: l + 1, items: byLayer.get(l)?.length ?? 0 } satisfies PlanPhaseData,
      width: PHASE_LABEL_W,
      height: 52,
      selectable: false,
      draggable: false,
    });
  }
  for (const [k, e] of artifacts) {
    const pos = placed.get(k);
    if (!pos) continue;
    nodes.push({
      id: `plan:${k}`,
      type: 'planArtifact',
      position: pos,
      data: {
        entity: e,
        sliceCount: sliceCount.get(k) ?? 0,
        scenarioCount: scenarioCount.get(k) ?? 0,
        status: trackingOf(model, e)[0]?.status,
      } satisfies PlanArtifactData,
    });
  }

  // Animation flows from the contract to whatever needs it: the direction
  // of the dependency.
  const edges: Edge[] = deps.map((d) => ({
    id: `dep:${d.from}->${d.to}`,
    source: `plan:${d.from}`,
    target: `plan:${d.to}`,
    animated: true,
    style: { stroke: '#a1a1aa', strokeWidth: 1.6 },
  }));

  return { nodes, edges, phases };
}

// Slice view: one slice at a time, drawn as a small lane by step board. The
// slice's own moments sit in the middle. One step to the left holds what
// feeds the slice, and one step to the right holds what the slice feeds, so
// a reader sees the slice in its causal context without the whole board.
//
// Links follow the board's edge grammar (layout.ts, the Studio README):
// ui -> command -> events in a command slice, events -> read model in a
// read model slice, read models -> processor -> command in an automation
// slice, read models -> ui in a UI slice. A neighbor is any card that
// another slice links to the slice's first step, or that another slice links
// from its last step. Matching is by entity identity, never by title.
//
// Everything here is a projection of the loaded model. Nothing decides
// whether the model is valid: findings come from the validator.

import type { Glyph } from './glyphs';
import {
  asId,
  entityKey,
  type EdgeRef,
  type Entity,
  type EntityId,
  type EntityKind,
  type FieldSpec,
  type Model,
  isDerivedField,
  latestSliceBySlug,
  orderedSlices,
  type ScenarioExample,
  type ScenarioView,
  type SliceView,
  slugKey,
  versionOf,
} from './model';

export type StepRole = 'upstream' | 'slice' | 'downstream';

export interface SliceCard {
  /** Unique on the board: the step and the entity, so one entity can stand at two moments. */
  id: string;
  kind: EntityKind;
  /** The identity the declaring slice names. */
  ref: EntityId;
  /** Undefined when the declaring slice names an entity that is not loaded. */
  entity?: Entity;
  step: number;
  role: StepRole;
  lane: string;
  /** The slice that declares this card: the moment it belongs to. */
  moment: Entity;
}

export interface SliceLink {
  id: string;
  source: string;
  target: string;
  /** The slice that declares the connection. */
  slice: Entity;
}

export interface SliceLane {
  id: string;
  label: string;
  glyph: Glyph;
  tint: string;
  /** Stream lanes stand for a swimlane entity. */
  entity?: Entity;
}

export interface SliceStep {
  index: number;
  role: StepRole;
}

export interface SliceNeighborhood {
  slice: SliceView;
  lanes: SliceLane[];
  steps: SliceStep[];
  cards: SliceCard[];
  links: SliceLink[];
}

interface Ref {
  kind: EntityKind;
  id: EntityId;
}

// One declared connection, as the board's edge grammar reads a slice.
interface DeclaredLink {
  from: Ref;
  to: Ref;
  slice: SliceView;
}

const refOf = (kind: EntityKind, edge: EdgeRef | undefined): Ref[] => (edge ? [{ kind, id: edge.id }] : []);
const refsOf = (kind: EntityKind, edges: EdgeRef[]): Ref[] => edges.map((e) => ({ kind, id: e.id }));
const refKey = (ref: Ref) => slugKey(ref.kind, ref.id);
const sliceSlug = (s: SliceView) => `${s.entity.id.namespace}/${s.entity.id.slug}`;

/** The slice's own moments in flow order. Empty steps are dropped. */
function ownSteps(slice: SliceView): Ref[][] {
  const steps: Ref[][] =
    slice.kind === 'commandSlice'
      ? [refOf('ui', slice.ui), refOf('command', slice.command), refsOf('event', slice.events)]
      : slice.kind === 'readModelSlice'
        ? [refsOf('event', slice.events), refOf('readModel', slice.readModel)]
        : slice.kind === 'automationSlice'
          ? [
              refsOf('readModel', slice.readModels),
              refOf('processor', slice.processor),
              refOf('command', slice.command),
            ]
          : [refsOf('readModel', slice.readModels), refOf('ui', slice.ui)];
  return steps.filter((step) => step.length > 0);
}

function declaredLinks(slice: SliceView): DeclaredLink[] {
  const steps = ownSteps(slice);
  const links: DeclaredLink[] = [];
  for (let i = 1; i < steps.length; i++) {
    for (const from of steps[i - 1]) for (const to of steps[i]) links.push({ from, to, slice });
  }
  return links;
}

/** Every entity reference the slice declares, the persona included. */
export function sliceMembers(slice: SliceView): Ref[] {
  return [...refOf('persona', slice.persona), ...ownSteps(slice).flat()];
}

const LANE = {
  automation: { id: 'automation', label: 'Automation', glyph: 'automation', tint: '#f5f3ff' },
  ui: { id: 'ui', label: 'UI', glyph: 'ui', tint: '#fefce8' },
  timeline: { id: 'timeline', label: 'Timeline', glyph: 'timeline', tint: '#eff6ff' },
  events: { id: 'events', label: 'Events', glyph: 'stream', tint: '#fff7ed' },
} as const satisfies Record<string, SliceLane>;

/**
 * The board for one slice: its own steps, plus one upstream and one
 * downstream step when other slices connect to it.
 */
export function sliceNeighborhood(model: Model, slice: SliceView): SliceNeighborhood {
  // Match layout.ts, sequence.ts and screens.ts: refs are (kind, slug), not
  // pinned version keys, so a slice that names @1 still shows the loaded @2.
  const bySlug = new Map<string, Entity>();
  for (const e of model.entities) {
    const k = slugKey(e.kind, e.id);
    const cur = bySlug.get(k);
    if (!cur || versionOf(e.id) > versionOf(cur.id)) bySlug.set(k, e);
  }
  const resolve = (ref: Ref) => bySlug.get(refKey(ref));
  const self = slugKey(slice.entity.kind, slice.entity.id);
  const others = [...latestSliceBySlug(model).values()].filter((s) => slugKey(s.entity.kind, s.entity.id) !== self);
  const otherLinks = others.flatMap(declaredLinks);

  const own = ownSteps(slice);
  const entry = own[0] ?? [];
  const exit = own[own.length - 1] ?? [];
  // A processor reads only what its own slice names: another automation
  // slice that feeds the same processor is not this slice's input, and a
  // processor with no command issues nothing downstream.
  const entryKeys = new Set(entry.filter((r) => r.kind !== 'processor').map(refKey));
  const exitKeys = new Set(exit.filter((r) => r.kind !== 'processor').map(refKey));
  const upstream = otherLinks.filter((l) => entryKeys.has(refKey(l.to)));
  const downstream = otherLinks.filter((l) => exitKeys.has(refKey(l.from)));

  const steps: { role: StepRole; refs: { ref: Ref; moment: Entity }[] }[] = [];
  const unique = (list: { ref: Ref; moment: Entity }[]) => {
    const seen = new Set<string>();
    return list.filter((x) => {
      const k = refKey(x.ref);
      if (seen.has(k)) return false;
      seen.add(k);
      return true;
    });
  };
  if (upstream.length > 0)
    steps.push({ role: 'upstream', refs: unique(upstream.map((l) => ({ ref: l.from, moment: l.slice.entity }))) });
  for (const step of own)
    steps.push({ role: 'slice', refs: unique(step.map((ref) => ({ ref, moment: slice.entity }))) });
  if (downstream.length > 0)
    steps.push({ role: 'downstream', refs: unique(downstream.map((l) => ({ ref: l.to, moment: l.slice.entity }))) });

  // Lanes in the board's order: automation on top, the persona's screens,
  // the command and read model timeline, then one lane per event stream.
  const streamLanes = new Map<string, SliceLane>();
  let hasLooseEvents = false;
  const laneOf = (ref: Ref, entity: Entity | undefined): string => {
    if (ref.kind === 'processor') return LANE.automation.id;
    if (ref.kind === 'ui') return LANE.ui.id;
    if (ref.kind === 'command' || ref.kind === 'readModel') return LANE.timeline.id;
    const lane = entity?.swimlane ? bySlug.get(slugKey('swimlane', entity.swimlane)) : undefined;
    if (!lane) {
      hasLooseEvents = true;
      return LANE.events.id;
    }
    const id = `swimlane:${slugKey('swimlane', lane.id)}`;
    if (!streamLanes.has(id))
      streamLanes.set(id, { id, label: lane.title, glyph: 'stream', tint: lane.color || '#fff7ed', entity: lane });
    return id;
  };

  const cards: SliceCard[] = [];
  const cardIdAt: Map<string, string>[] = [];
  steps.forEach((step, index) => {
    const ids = new Map<string, string>();
    for (const { ref, moment } of step.refs) {
      const entity = resolve(ref);
      const id = `${index}:${refKey(ref)}`;
      ids.set(refKey(ref), id);
      cards.push({
        id,
        kind: ref.kind,
        ref: ref.id,
        entity,
        step: index,
        role: step.role,
        lane: laneOf(ref, entity),
        moment,
      });
    }
    cardIdAt.push(ids);
  });

  const links: SliceLink[] = [];
  const seenLinks = new Set<string>();
  const link = (fromStep: number, from: Ref, to: Ref, declaredBy: Entity) => {
    const source = cardIdAt[fromStep]?.get(refKey(from));
    const target = cardIdAt[fromStep + 1]?.get(refKey(to));
    if (!source || !target) return;
    const id = `${source}->${target}`;
    if (seenLinks.has(id)) return;
    seenLinks.add(id);
    links.push({ id, source, target, slice: declaredBy });
  };
  const firstOwn = upstream.length > 0 ? 1 : 0;
  for (const l of upstream) link(0, l.from, l.to, l.slice.entity);
  for (const l of declaredLinks(slice)) {
    const offset = own.findIndex((step) => step.some((r) => refKey(r) === refKey(l.from)));
    link(firstOwn + offset, l.from, l.to, slice.entity);
  }
  const lastOwn = firstOwn + own.length - 1;
  for (const l of downstream) link(lastOwn, l.from, l.to, l.slice.entity);

  const lanes: SliceLane[] = [];
  const used = new Set(cards.map((c) => c.lane));
  for (const lane of [LANE.automation, LANE.ui, LANE.timeline]) if (used.has(lane.id)) lanes.push(lane);
  lanes.push(
    ...[...streamLanes.values()].sort(
      (a, b) => (a.entity?.order ?? 0) - (b.entity?.order ?? 0) || a.label.localeCompare(b.label),
    ),
  );
  if (hasLooseEvents) lanes.push(LANE.events);

  return { slice, lanes, steps: steps.map((s, index) => ({ index, role: s.role })), cards, links };
}

// ---- geometry ---------------------------------------------------------------

export const SLICE_CARD_W = 184;
export const SLICE_CARD_H = 96;
const STEP_GAP = 72;
const STEP_W = SLICE_CARD_W + STEP_GAP;
const LANE_LABEL_W = 112;
const LANE_PAD = 12;
const LANE_GAP = 6;
const STACK_GAP = 12;
const HEADER_H = 44;
const EDGE_PAD = 16;

export interface Point {
  x: number;
  y: number;
}

export interface SliceBoardGeometry {
  width: number;
  height: number;
  lanes: { lane: SliceLane; y: number; height: number }[];
  steps: { step: SliceStep; x: number }[];
  cards: Map<string, Point>;
  links: Map<string, { d: string; mid: Point }>;
}

/** Fixed geometry for the board: cards never move, so the layout is a pure function of the neighborhood. */
export function sliceBoardGeometry(hood: SliceNeighborhood): SliceBoardGeometry {
  const stackIndex = new Map<string, number>();
  const depthByLane = new Map<string, number>();
  const counts = new Map<string, number>();
  for (const card of hood.cards) {
    const k = `${card.lane}\0${card.step}`;
    const n = counts.get(k) ?? 0;
    stackIndex.set(card.id, n);
    counts.set(k, n + 1);
    depthByLane.set(card.lane, Math.max(depthByLane.get(card.lane) ?? 0, n + 1));
  }
  const stepX = (step: number) => LANE_LABEL_W + EDGE_PAD + step * STEP_W;
  const lanes: SliceBoardGeometry['lanes'] = [];
  let y = HEADER_H;
  for (const lane of hood.lanes) {
    const depth = depthByLane.get(lane.id) ?? 1;
    const height = depth * (SLICE_CARD_H + STACK_GAP) - STACK_GAP + LANE_PAD * 2;
    lanes.push({ lane, y, height });
    y += height + LANE_GAP;
  }
  const laneY = new Map(lanes.map((l) => [l.lane.id, l.y]));
  const cards = new Map<string, Point>();
  for (const card of hood.cards) {
    cards.set(card.id, {
      x: stepX(card.step),
      y: (laneY.get(card.lane) ?? HEADER_H) + LANE_PAD + (stackIndex.get(card.id) ?? 0) * (SLICE_CARD_H + STACK_GAP),
    });
  }
  const links = new Map<string, { d: string; mid: Point }>();
  for (const l of hood.links) {
    const s = cards.get(l.source);
    const t = cards.get(l.target);
    if (!s || !t) continue;
    const p0 = { x: s.x + SLICE_CARD_W, y: s.y + SLICE_CARD_H / 2 };
    const p3 = { x: t.x, y: t.y + SLICE_CARD_H / 2 };
    const dx = (p3.x - p0.x) / 2;
    const p1 = { x: p0.x + dx, y: p0.y };
    const p2 = { x: p3.x - dx, y: p3.y };
    links.set(l.id, {
      d: `M${p0.x},${p0.y} C${p1.x},${p1.y} ${p2.x},${p2.y} ${p3.x},${p3.y}`,
      // The cubic Bezier point at t = 0.5.
      mid: { x: (p0.x + 3 * p1.x + 3 * p2.x + p3.x) / 8, y: (p0.y + 3 * p1.y + 3 * p2.y + p3.y) / 8 },
    });
  }
  const steps = hood.steps.map((step) => ({ step, x: stepX(step.index) }));
  const width = stepX(Math.max(hood.steps.length, 1)) - STEP_GAP + EDGE_PAD;
  return { width, height: y - LANE_GAP + EDGE_PAD, lanes, steps, cards, links };
}

// ---- field flow ---------------------------------------------------------------

// The three kinds that declare a schema. Processors and UIs carry no fields
// of their own: data passes through them.
const CARRIER_KINDS: ReadonlySet<EntityKind> = new Set(['event', 'command', 'readModel']);

/**
 * How one card relates to a traced field name:
 *   carries   the entity's schema has a top-level field with that name
 *   state     an event whose swimlane state holds the name: the stream
 *             keeps the value, and the validator counts it as carried
 *   passes    a processor or UI next to a card that carries the name
 *   lacks     the entity declares a schema without that name
 *   none      a processor or UI the field does not touch
 *   unloaded  the declaring slice names an entity that is not loaded
 */
export type FieldPresence = 'carries' | 'state' | 'passes' | 'lacks' | 'none' | 'unloaded';

export interface FieldFlow {
  name: string;
  presence: Map<string, FieldPresence>;
  /** Links whose both ends hold the field. */
  carried: Set<string>;
  /** Links where the source holds the field and the target's schema does not. */
  stops: Set<string>;
}

const HELD: ReadonlySet<FieldPresence> = new Set(['carries', 'state']);
const ON: ReadonlySet<FieldPresence> = new Set(['carries', 'state', 'passes']);

export function isFieldCarrier(kind: EntityKind): boolean {
  return CARRIER_KINDS.has(kind);
}

export function fieldOf(entity: Entity | undefined, name: string): FieldSpec | undefined {
  return entity?.fields.find((f) => f.name === name);
}

/** The swimlane state field an event's stream keeps under `name`, if any. */
export function streamStateField(model: Model, event: Entity | undefined, name: string): FieldSpec | undefined {
  if (!event || event.kind !== 'event' || !event.swimlane) return undefined;
  const lane = model.byKey.get(entityKey('swimlane', event.swimlane));
  const state = Array.isArray(lane?.raw.state) ? (lane.raw.state as FieldSpec[]) : [];
  return state.find((f) => f.name === name);
}

/**
 * Follows one field name along the board's links. Names match only along a
 * declared link, the same way the validator's field rules read the model
 * (EVENT_FIELD_UNSOURCED, READ_MODEL_FIELD_UNSOURCED and the others): two
 * unrelated entities that share a name are not data flow. Only top-level
 * schema fields count, as in the validator.
 */
export function fieldFlow(model: Model, hood: SliceNeighborhood, name: string): FieldFlow {
  const presence = new Map<string, FieldPresence>();
  for (const card of hood.cards) {
    if (!card.entity) presence.set(card.id, 'unloaded');
    else if (!CARRIER_KINDS.has(card.kind)) presence.set(card.id, 'none');
    else if (fieldOf(card.entity, name)) presence.set(card.id, 'carries');
    else if (streamStateField(model, card.entity, name)) presence.set(card.id, 'state');
    else presence.set(card.id, 'lacks');
  }
  // A processor or UI passes the field on only when it arrives from the
  // inbound side. A field that only the outbound card holds has no source.
  const held = (id: string) => HELD.has(presence.get(id) ?? 'none');
  // The validator does not report a derived field as unsourced, so neither does the trace.
  const derivedOn = (id: string) => {
    const f = fieldOf(hood.cards.find((c) => c.id === id)?.entity, name);
    return Boolean(f && isDerivedField(f));
  };
  const unsourced = new Set<string>();
  for (const card of hood.cards) {
    if (presence.get(card.id) !== 'none') continue;
    if (hood.links.some((l) => l.target === card.id && held(l.source))) presence.set(card.id, 'passes');
    else if (
      card.kind === 'processor' &&
      card.role === 'slice' &&
      hood.links.some((l) => l.source === card.id && held(l.target) && !derivedOn(l.target))
    )
      unsourced.add(card.id);
  }
  const carried = new Set<string>();
  const stops = new Set<string>();
  for (const l of hood.links) {
    const from = presence.get(l.source) ?? 'none';
    const to = presence.get(l.target) ?? 'none';
    if (ON.has(from) && ON.has(to)) carried.add(l.id);
    else if (ON.has(from) && to === 'lacks') stops.add(l.id);
    else if (unsourced.has(l.target)) stops.add(l.id);
  }
  return { name, presence, carried, stops };
}

/** Field names declared on the board's cards, in board order: step, then lane, then stack. */
export function boardFieldNames(hood: SliceNeighborhood): string[] {
  const names: string[] = [];
  const seen = new Set<string>();
  for (const card of boardOrder(hood)) {
    if (!CARRIER_KINDS.has(card.kind)) continue;
    for (const f of card.entity?.fields ?? []) {
      const n = typeof f.name === 'string' ? f.name : '';
      if (!n || seen.has(n)) continue;
      seen.add(n);
      names.push(n);
    }
  }
  return names;
}

/** Cards in reading order: left to right, then top to bottom. */
export function boardOrder(hood: SliceNeighborhood): SliceCard[] {
  const laneRank = new Map(hood.lanes.map((l, i) => [l.id, i]));
  return hood.cards
    .map((card, i) => ({ card, i }))
    .sort(
      (a, b) =>
        a.card.step - b.card.step || (laneRank.get(a.card.lane) ?? 0) - (laneRank.get(b.card.lane) ?? 0) || a.i - b.i,
    )
    .map((x) => x.card);
}

// ---- picking a slice -----------------------------------------------------------

/** The slice's `?slice=` value: its namespace and slug, as orderedSlices keys it. */
export function sliceParam(slice: SliceView): string {
  return sliceSlug(slice);
}

export function sliceByParam(model: Model, param: string | null | undefined): SliceView | undefined {
  if (!param) return undefined;
  return orderedSlices(model).find((s) => sliceSlug(s) === param);
}

const SLICE_KINDS: ReadonlySet<EntityKind> = new Set(['commandSlice', 'readModelSlice', 'automationSlice', 'uiSlice']);

/**
 * The slice to open for an entity: the entity itself when it is a slice,
 * otherwise the first slice in storyboard order that names it.
 */
export function sliceForEntity(model: Model, key: string | undefined): SliceView | undefined {
  const entity = key ? model.byKey.get(key) : undefined;
  if (!entity) return undefined;
  if (SLICE_KINDS.has(entity.kind)) {
    return latestSliceBySlug(model).get(`${entity.id.namespace}/${entity.id.slug}`);
  }
  const target = slugKey(entity.kind, entity.id);
  return orderedSlices(model).find((s) => sliceMembers(s).some((m) => refKey(m) === target));
}

export interface SliceGroup {
  /** Undefined for slices no storyboard lists. */
  storyboard?: Entity;
  slices: SliceView[];
}

/** Picker groups: one per storyboard in model order, each slice under the first storyboard that lists it. */
export function sliceGroups(model: Model): SliceGroup[] {
  const bySlug = latestSliceBySlug(model);
  const claimed = new Set<string>();
  const groups: SliceGroup[] = [];
  for (const sb of model.storyboards) {
    const slices: SliceView[] = [];
    for (const ref of Array.isArray(sb.raw.slices) ? sb.raw.slices : []) {
      const id = asId((ref as Record<string, unknown>)?.id);
      const k = `${id.namespace}/${id.slug}`;
      const slice = bySlug.get(k);
      if (!slice || claimed.has(k)) continue;
      claimed.add(k);
      slices.push(slice);
    }
    if (slices.length > 0) groups.push({ storyboard: sb, slices });
  }
  const rest = orderedSlices(model).filter((s) => !claimed.has(sliceSlug(s)));
  if (rest.length > 0) groups.push({ slices: rest });
  return groups;
}

// ---- story playback --------------------------------------------------------------

/** Storyboards that list the slice, in model order. */
export function storyboardsOf(model: Model, slice: SliceView): Entity[] {
  const k = sliceSlug(slice);
  return model.storyboards.filter((sb) =>
    (Array.isArray(sb.raw.slices) ? sb.raw.slices : []).some((ref) => {
      const id = asId((ref as Record<string, unknown>)?.id);
      return `${id.namespace}/${id.slug}` === k;
    }),
  );
}

export interface StorySteps {
  /** The storyboard's slices in its own order. A repeated slice is a new moment and stays repeated. */
  slices: SliceView[];
  /** Listed slices that are not loaded, so playback skips them. */
  missing: EntityId[];
}

export function storySteps(model: Model, storyboard: Entity): StorySteps {
  const bySlug = latestSliceBySlug(model);
  const slices: SliceView[] = [];
  const missing: EntityId[] = [];
  for (const ref of Array.isArray(storyboard.raw.slices) ? storyboard.raw.slices : []) {
    const id = asId((ref as Record<string, unknown>)?.id);
    const slice = bySlug.get(`${id.namespace}/${id.slug}`);
    if (slice) slices.push(slice);
    else if (id.slug) missing.push(id);
  }
  return { slices, missing };
}

// ---- scenario playback ------------------------------------------------------------

export type ScenarioPhase = 'given' | 'when' | 'then';

export interface ScenarioStep {
  phase: ScenarioPhase;
  examples: ScenarioExample[];
  /** Automation slices state the trigger in prose. */
  doc?: string;
  reject?: { reasonCode: string; doc: string };
}

/**
 * A scenario as given, when, then steps, in the Inspector's terms. A read
 * model slice has no precondition, so it has no given step. An empty given
 * stays: "given nothing" says the moment starts from no prior state.
 */
export function scenarioSteps(slice: SliceView, scenario: ScenarioView): ScenarioStep[] {
  const steps: ScenarioStep[] = [];
  if (slice.kind !== 'readModelSlice') steps.push({ phase: 'given', examples: scenario.given });
  if (scenario.when.length > 0 || scenario.whenDoc)
    steps.push({ phase: 'when', examples: scenario.when, doc: scenario.whenDoc });
  steps.push({
    phase: 'then',
    examples: [
      ...scenario.thenEmits,
      ...(scenario.thenState ? [scenario.thenState] : []),
      ...(scenario.thenCommand ? [scenario.thenCommand] : []),
    ],
    reject: scenario.reject,
  });
  return steps;
}

/** Version-free keys of the entities a step names, to match against board cards. */
export function scenarioStepKeys(step: ScenarioStep): Set<string> {
  return new Set(step.examples.map((e) => slugKey(e.kind, e.id)));
}

/** Version-free key of a card, to match scenario examples and focus requests. */
export function cardSlugKey(card: SliceCard): string {
  return slugKey(card.kind, card.ref);
}

// Classic Event Modeling board layout: automation on top, one swimlane per
// persona holding that persona's UI screens, the command/read-model timeline
// in the middle, and one event stream lane per swimlane below. Columns follow
// storyboard slice order, so the board reads left-to-right as time.
//
// Stickies are temporal OCCURRENCES of an entity, not graph nodes: commands,
// processors and events happen at one moment and appear once, while UI screens
// and read models live on and appear once per slice that shows them. Every
// edge points forward in time: an event always feeds the next occurrence of
// a read model, never an earlier one. When a read model has no occurrence
// after an event that updates it, a synthesized occurrence is placed between
// columns so the update stays visible without pointing backward.
//
// ARCHITECTURE
//
// `layoutBoard` is a thin orchestrator. The work is split across:
//   * `OccurrenceMap` owns the per-(entity, column) placement state and
//     exposes the four primitives every pass uses: `createOcc`, `place`,
//     `at`, `latestBefore`.
//   * `buildLaneMap`: derives the lane list (automation / personas / UI /
//     timeline / swimlanes) and the per-entity `laneFor` mapping.
//   * Placement passes: `placeSlicePrimaries`, `placeConsumerSources`,
//     `placeReferencedFallbacks`, `placeStoryboardEntries`,
//     `placeUnwiredEntities`.
//   * `resolveRmLinks`: synthesizes forward-only event → read-model edges.
//   * `buildVisualLayout`: turns the abstract occurrence map into geometry
//     (laneY, colSlots, slotOf, boardW/H).
//   * `buildHeaderNodes`, `buildStickyNodes`, `buildEdges`: emit the
//     ReactFlow nodes / edges.

import type { Edge, Node } from '@xyflow/react';
import type { Glyph } from './glyphs';
import {
  asId,
  commandsWithMultipleIssuers,
  commandsWithoutEmittedEvents,
  type EdgeRef,
  type Entity,
  type EntityId,
  type EntityKind,
  instanceUidOf,
  type Model,
  orderedSlices,
  slugKey,
  staleRefsOf,
  storyboardEntryObserver,
  storyboardEntryReadModel,
  trackingOf,
  versionOf,
} from './model';

export const NODE_W = 216;
// Fixed sticky height: every card is the same size whether or not it has a
// doc, uniform cards read as one timeline; missing data is just whitespace.
export const NODE_H = 122;
const COL_W = 264;
const ROW_GAP = 24;
const ROW_H = NODE_H + ROW_GAP;
const LANE_PAD = 28;
const LANE_LABEL_W = 224;
const LANE_GAP = 18;

// A lane wraps `depth` rows of cards plus the gutters BETWEEN them: the last
// row has no trailing gutter, so cards stay vertically centered in the lane.
const laneHeight = (depth: number) => depth * ROW_H - ROW_GAP + LANE_PAD * 2;

interface Lane {
  id: string;
  label: string;
  glyph: Glyph;
  tint: string;
  // Persona and stream lanes ARE entities; clicking the lane selects them.
  entity?: Entity;
}

export interface LaneData extends Record<string, unknown> {
  label: string;
  glyph?: Glyph;
  tint: string;
  entity?: Entity;
  // Selection is carried in data, NOT via ReactFlow's selected flag: lanes
  // are background chrome and must never be elevated above stickies.
  selected?: boolean;
}

interface Occurrence {
  id: string;
  entity: Entity;
  lane: number;
  col: number;
  stack: number;
  synthetic?: boolean;
}

export interface StickyData extends Record<string, unknown> {
  entity: Entity;
  synthetic?: boolean;
  // Instance coordinate: "#2/4", position of this moment among the type's
  // occurrences IN THIS VIEW (a reading aid, not an identity).
  instance?: string;
  // Instance IDENTITY: the slice that defines this moment. A slice captures
  // one temporal moment, so type x defining slice names the instance
  // stably, independent of any view. Synthesized occurrences have none.
  moment?: Entity;
  // uuidv5(type uid / defining-slice uid): the machine id of THIS moment.
  instanceUid?: string;
  // Forks shown from the observed state: this occurrence is the view a
  // branch's entry watches, so a road opens here.
  roads?: { title: string; doc: string }[];
  // Validation flag mirrored from the server's COMMAND_NO_EMITTED_EVENTS rule:
  // this command has no CommandSlice that emits any events. Set only on
  // command stickies; renderer paints it red with a "no events" badge.
  noEmittedEvents?: boolean;
  // COMMAND_MULTIPLE_ISSUERS: this command is issued by more than one UI or
  // processor across the model; the trigger is ambiguous. Set only on
  // command stickies; renderer paints it red with an error badge.
  multipleIssuers?: boolean;
  // Branch review status (Phase 3: Studio) stamped by matching this
  // sticky's entity against a `/api/branch-diff` lookup at the board level
  // (see `applyBranchStatus` in branch.ts); never computed inside
  // `layoutBoard` itself. One of "added" | "changed" | "conflict"; absent
  // when the entity has no corresponding diff entry (baseline, or
  // unchanged on the branch).
  branchStatus?: string;
}

export interface SliceHeaderData extends Record<string, unknown> {
  entity: Entity;
  sliceKind: string;
  // Delivery status from tracker overlays (Decision #25): never from the
  // slice itself.
  status?: string;
  // Count of refs on this slice pinning an older version than what's loaded
  // (Decision #24 mid-flight signal). Drives the canvas migration-pending
  // chip; the drawer carries the full per-ref breakdown.
  staleRefs?: number;
}

export interface GapHeaderData extends Record<string, unknown> {
  events: string[];
  readModels: string[];
}

// Connection payload for the inspector: the proto Edge's design data plus
// the endpoints and the slice that declared the connection.
export interface BoardEdgeData extends Record<string, unknown> {
  relation: string;
  doc: string;
  metadata: unknown[];
  source: Entity;
  target: Entity;
  slice?: Entity;
}

// UI screens and read models persist over time, so they repeat per slice.
// Processors repeat too: the same automation can be triggered by more than
// one slice, and each trigger is its own moment on the board.
const MULTI_OCCURRENCE: ReadonlySet<EntityKind> = new Set(['ui', 'readModel', 'processor']);

type Slice = ReturnType<typeof orderedSlices>[number];

// =============================================================================
// OccurrenceMap: owns placement state for every pass.
//
// Two reasons this is a class: the state (occurrences, occsByEntity,
// stackDepth) is mutated across half a dozen passes that would otherwise
// each need to thread the maps as parameters; and the four primitives
// (`createOcc`, `place`, `at`, `latestBefore`) want shared coordinate
// invariants (sub-column fanout, MULTI_OCCURRENCE handling) in one place.
// =============================================================================
class OccurrenceMap {
  readonly occurrences = new Map<string, Occurrence>();
  readonly occsByEntity = new Map<string, Occurrence[]>();
  // `${lane}:${col}:${stack}`; slot reservation for stacking entries in
  // the same lane+column. Read at the very end to derive lane heights.
  readonly stackDepth = new Map<string, number>();

  constructor(
    private readonly bySlug: Map<string, Entity>,
    private readonly laneFor: (entity: Entity) => number,
  ) {}

  /** Create a fresh occurrence at the given column. Returns undefined for
   *  entities with no lane (e.g. swimlanes / event models). MULTI_OCCURRENCE
   *  kinds (UI, read model, processor) get a column-qualified id so the same
   *  entity may appear in many columns; everything else collapses to one. */
  createOcc(entity: Entity, col: number): Occurrence | undefined {
    const lane = this.laneFor(entity);
    if (lane < 0) return undefined;
    const id = MULTI_OCCURRENCE.has(entity.kind) ? `${entity.key}@${col}` : entity.key;
    const existing = this.occurrences.get(id);
    if (existing) return existing;
    // One moment, one column. A second occupant of the same lane+column
    // stacks VERTICALLY (events emitted by the same command share the
    // command's X; the second event sits below the first), so reading down
    // a column shows everything that happened at that moment.
    let stack = 0;
    while (this.stackDepth.has(`${lane}:${col}:${stack}`)) stack++;
    this.stackDepth.set(`${lane}:${col}:${stack}`, 1);
    const occ: Occurrence = { id, entity, lane, col, stack };
    this.occurrences.set(id, occ);
    const list = this.occsByEntity.get(entity.key) ?? [];
    list.push(occ);
    this.occsByEntity.set(entity.key, list);
    return occ;
  }

  /** Look up the entity for `ref` and place a new occurrence (or return the
   *  single existing one for non-MULTI_OCCURRENCE kinds). */
  place(kind: EntityKind, ref: EntityId | undefined, col: number): Occurrence | undefined {
    if (!ref) return undefined;
    const entity = this.bySlug.get(slugKey(kind, ref));
    if (!entity) return undefined;
    if (!MULTI_OCCURRENCE.has(kind)) {
      const existing = this.occsByEntity.get(entity.key)?.[0];
      if (existing) return existing;
    }
    return this.createOcc(entity, col);
  }

  /** Existing occurrence: the slice's column (incl. sub-columns) for repeating kinds, the single one otherwise. */
  at(kind: EntityKind, ref: EntityId | undefined, col?: number): Occurrence | undefined {
    if (!ref) return undefined;
    const entity = this.bySlug.get(slugKey(kind, ref));
    if (!entity) return undefined;
    const list = this.occsByEntity.get(entity.key) ?? [];
    if (!MULTI_OCCURRENCE.has(kind)) return list[0];
    if (col === undefined) return undefined;
    return list.find((o) => o.col >= col && o.col < col + 0.4);
  }

  /** Most recent occurrence at or before this column: "the latest state". */
  latestBefore(kind: EntityKind, ref: EntityId | undefined, col: number): Occurrence | undefined {
    if (!ref) return undefined;
    const entity = this.bySlug.get(slugKey(kind, ref));
    if (!entity) return undefined;
    const list = (this.occsByEntity.get(entity.key) ?? [])
      .filter((o) => o.col < col + 0.4)
      .sort((a, b) => a.col - b.col);
    return list[list.length - 1];
  }
}

// =============================================================================
// Pass 0: build the latest-version index used by every slice ref lookup.
// =============================================================================
function buildLatestBySlug(model: Model): Map<string, Entity> {
  const bySlug = new Map<string, Entity>();
  for (const e of model.entities) {
    const k = slugKey(e.kind, e.id);
    const cur = bySlug.get(k);
    if (!cur || versionOf(e.id) > versionOf(cur.id)) bySlug.set(k, e);
  }
  return bySlug;
}

// =============================================================================
// Lane ids.
//
// A lane id is a string only because it becomes a ReactFlow node id. There are
// exactly TWO families and the PREFIX declares which, so the families can
// never overlap no matter what a modeler types:
//
//   `entity:<kind>:<namespace>/<slug>`  a lane standing for a real entity
//   `reserved:<name>`                   a lane the layout itself invents
//
// Nothing is inferred from the shape of a slug. Before this split the ids were
// bare (`stream:<slug>` next to a literal `stream:unassigned`), which collided
// three ways: a swimlane slugged `unassigned` shadowed the catch-all, the same
// slug in two namespaces produced one id for two lanes, and two versions of
// one swimlane produced the id twice.
// =============================================================================
const LaneId = {
  forEntity: (kind: EntityKind, id: EntityId): string => `entity:${slugKey(kind, id)}`,
  reserved: (name: string): string => `reserved:${name}`,
} as const;

// =============================================================================
// The ordered lane list. Push is IDEMPOTENT on lane id: a repeat returns the
// index already assigned instead of emitting a second lane, so a duplicate can
// never reach ReactFlow as two nodes sharing one node id. With the id families
// above a repeat should be unreachable; this is the net for the change nobody
// predicted.
// =============================================================================
class LaneList {
  private readonly items: Lane[] = [];
  private readonly indexById = new Map<string, number>();

  push(lane: Lane): number {
    const existing = this.indexById.get(lane.id);
    if (existing !== undefined) return existing;
    const index = this.items.length;
    this.indexById.set(lane.id, index);
    this.items.push(lane);
    return index;
  }

  toArray(): Lane[] {
    return this.items;
  }
}

// =============================================================================
// Lane lookup for one keyed family of lanes: personas (which own UI screens)
// or swimlanes (which own events).
//
// Two reasons this is a class rather than a bare Map. The catch-all lane
// (`ui:unassigned` / `stream:unassigned`, the lane a stray falls into) is held
// in its own field instead of under a reserved key, so the key space contains
// nothing but real entity references and no slug a modeler picks can shadow a
// lane. And key construction lives in exactly one place, so no caller can look
// up with a differently-shaped key than the one that was inserted.
// =============================================================================
class LaneIndex {
  private readonly byEntity = new Map<string, number>();
  private catchAll?: number;

  constructor(private readonly kind: EntityKind) {}

  assign(id: EntityId, lane: number): void {
    this.byEntity.set(slugKey(this.kind, id), lane);
  }

  assignCatchAll(lane: number): void {
    this.catchAll = lane;
  }

  /** The lane that OWNS `id`, or undefined when nothing does. Deliberately
   *  does not fall back to the catch-all: deciding whether the catch-all is
   *  needed at all is exactly the "is anything stray?" question. */
  owner(id: EntityId | undefined): number | undefined {
    return id ? this.byEntity.get(slugKey(this.kind, id)) : undefined;
  }

  /** The lane to DRAW `id` in: its owner, else the catch-all, else off-board. */
  resolve(id: EntityId | undefined): number {
    return this.owner(id) ?? this.catchAll ?? -1;
  }
}

// =============================================================================
// Pass 1: lane construction.
//
// Returns the lane list plus a `laneFor(entity)` lookup. Lane indices are
// embedded in every Occurrence and used downstream for Y coordinates.
// =============================================================================
interface LaneMap {
  lanes: Lane[];
  laneFor: (entity: Entity) => number;
}

function buildLaneMap(model: Model, bySlug: Map<string, Entity>): LaneMap {
  const personas = [...bySlug.values()]
    .filter((e) => e.kind === 'persona')
    .sort((a, b) => (a.order ?? 0) - (b.order ?? 0) || a.title.localeCompare(b.title));

  // A UI screen belongs to the persona that operates it. `Ui` carries no
  // owner of its own, so attribution is inferred from the three places the
  // schema pairs a persona WITH a ui:
  //   1. UiSlice.persona + .ui: the persona watches there.
  //   2. CommandSlice.persona + .ui: the persona issues a command there.
  //   3. StoryboardEntry.HumanObserver: the persona watches that screen.
  // (1) is what makes a display-only screen attributable on its own; the
  // storyboard entry is a fallback, no longer the only route. This
  // definition is mirrored EXACTLY by the server's UI_NO_PERSONA rule; the
  // lane below and that error must appear together or the board and the
  // validator disagree.
  const uiPersona = new Map<string, EntityId>();
  const attribute = (ui?: EntityId, persona?: EntityId) => {
    if (!ui || !persona) return;
    uiPersona.set(slugKey('ui', ui), persona);
  };
  for (const slice of model.slices) attribute(slice.ui?.id, slice.persona?.id);
  for (const sb of model.storyboards) {
    const observer = storyboardEntryObserver(sb);
    attribute(observer.ui, observer.persona);
  }

  const lanes = new LaneList();
  const automationLane = lanes.push({
    id: LaneId.reserved('automation'),
    label: 'Automation',
    glyph: 'automation',
    tint: '#f5f3ff',
  });
  const personaLaneIndex = new LaneIndex('persona');
  for (const p of personas) {
    personaLaneIndex.assign(
      p.id,
      lanes.push({
        id: LaneId.forEntity('persona', p.id),
        label: p.title,
        glyph: 'persona',
        tint: '#fefce8',
        entity: p,
      }),
    );
  }
  // Catch-all for screens no persona owns, the exact counterpart of
  // `stream:unassigned` below: created ONLY when such a screen exists, and
  // rendering a state the server rejects (UI_NO_PERSONA, Error). It is a
  // window onto a temporarily incomplete model, never a legitimate home.
  const hasLooseUi = [...bySlug.values()].some(
    (e) => e.kind === 'ui' && personaLaneIndex.owner(uiPersona.get(slugKey('ui', e.id))) === undefined,
  );
  if (hasLooseUi) {
    personaLaneIndex.assignCatchAll(
      lanes.push({ id: LaneId.reserved('ui:unassigned'), label: 'UI', glyph: 'ui', tint: '#fafafa' }),
    );
  }
  const timelineLane = lanes.push({
    id: LaneId.reserved('timeline'),
    label: 'Timeline',
    glyph: 'timeline',
    tint: '#eff6ff',
  });

  // Read from `bySlug`, not `model.swimlanes`: the latter is an unfiltered
  // kind filter, so a store holding two versions of one swimlane would draw
  // the lane twice. Personas already came from the deduped map.
  const swimlaneLaneIndex = new LaneIndex('swimlane');
  const swimlaneEntities = [...bySlug.values()]
    .filter((e) => e.kind === 'swimlane')
    .sort((a, b) => (a.order ?? 0) - (b.order ?? 0));
  for (const lane of swimlaneEntities) {
    swimlaneLaneIndex.assign(
      lane.id,
      lanes.push({
        id: LaneId.forEntity('swimlane', lane.id),
        label: lane.title,
        glyph: 'stream',
        tint: lane.color || '#fff7ed',
        entity: lane,
      }),
    );
  }
  const hasLooseEvents = model.entities.some(
    (e) => e.kind === 'event' && swimlaneLaneIndex.owner(e.swimlane) === undefined,
  );
  if (hasLooseEvents) {
    swimlaneLaneIndex.assignCatchAll(
      lanes.push({ id: LaneId.reserved('stream:unassigned'), label: 'Events', glyph: 'stream', tint: '#fff7ed' }),
    );
  }

  const laneFor = (entity: Entity): number => {
    switch (entity.kind) {
      case 'processor':
        return automationLane;
      case 'ui':
        return personaLaneIndex.resolve(uiPersona.get(slugKey('ui', entity.id)));
      case 'command':
      case 'readModel':
        return timelineLane;
      case 'event':
        return swimlaneLaneIndex.resolve(entity.swimlane);
      default:
        return -1;
    }
  };

  return { lanes: lanes.toArray(), laneFor };
}

// =============================================================================
// Pass 2: primary slice placement.
//
// Each slice owns a column. Point-in-time entities (commands, processors,
// events) sit in the column of the slice that defines them; UI screens and
// read models get a fresh occurrence in every slice that shows them.
// Co-emitted events fan side-by-side at fractional sub-columns of
// the emitting slice's column (base + j*0.001).
// =============================================================================
function placeSlicePrimaries(occs: OccurrenceMap, slices: Slice[]): void {
  slices.forEach((slice, col) => {
    if (slice.kind === 'commandSlice') {
      occs.place('ui', slice.ui?.id, col);
      occs.place('command', slice.command?.id, col);
      slice.events.forEach((ev, j) => {
        occs.place('event', ev.id, col + j * 0.001);
      });
    }
    if (slice.kind === 'readModelSlice') {
      occs.place('readModel', slice.readModel?.id, col);
    }
    if (slice.kind === 'automationSlice') {
      // Read model (col), processor (col + 0.001), command (col + 0.002) lay
      // out left-to-right within the slice: without distinct sub-columns the
      // read model and emitted command share the timeline lane's X and stack.
      occs.place('processor', slice.processor?.id, col + 0.001);
    }
    if (slice.kind === 'uiSlice') {
      // The screen sits where the processor does in an automation: right of
      // the read models it renders, so the arrows read left-to-right.
      occs.place('ui', slice.ui?.id, col + 0.001);
    }
  });
}

// =============================================================================
// Pass 3: consumer source read-models.
//
// A consumer (an automation handing read models to a processor, a UI slice
// handing them to a person) sees the LATEST state of its sources: reuse the
// most recent occurrence to the left instead of duplicating the sticky. Only
// when no occurrence exists yet does the consumer's column create one.
// Multiple sources fan side-by-side at fractional sub-columns (like
// co-emitted events), staying left of the consumer (+0.001) and, for an
// automation, its emitted command (+0.002).
// =============================================================================
function placeConsumerSources(occs: OccurrenceMap, slices: Slice[]): void {
  slices.forEach((slice, col) => {
    if (slice.kind !== 'automationSlice' && slice.kind !== 'uiSlice') return;
    // Stay strictly left of the consumer at col+0.001: with step 0.0001,
    // the 11th source lands on it. Fit n sources into (0, 0.001).
    const step = 0.001 / (slice.readModels.length + 1);
    slice.readModels.forEach((rm, j) => {
      if (!occs.latestBefore('readModel', rm.id, col)) occs.place('readModel', rm.id, col + (j + 1) * step);
    });
  });
}

// =============================================================================
// Pass 4: fallback placement for referenced-but-undefined entities.
//
// Point-in-time entities referenced but never defined by a slice (e.g. an
// automation's emitted command without its own slice, external events) still
// need a column: first reference wins. Co-emitted events from a commandSlice
// fan out at fractional sub-columns; readModelSlice's source events reuse
// existing occurrences and fall back to the slice column.
// =============================================================================
function placeReferencedFallbacks(occs: OccurrenceMap, slices: Slice[]): void {
  slices.forEach((slice, col) => {
    const commandCol = slice.kind === 'automationSlice' ? col + 0.002 : col;
    occs.place('command', slice.command?.id, commandCol);
    if (slice.kind === 'commandSlice') {
      slice.events.forEach((ev, j) => {
        occs.place('event', ev.id, col + j * 0.001);
      });
    } else {
      for (const ev of slice.events) occs.place('event', ev.id, col);
    }
  });
}

// =============================================================================
// Pass 5: storyboard entry observers.
//
// A storyboard's entry observer (the persona watching, the UI they observe
// through, the automation processor that triggers the flow, and the read
// model that anchors the moment) belongs at the storyboard's first column.
//
// ONLY place an entity here if no slice already positioned it: otherwise a
// storyboard whose entry observes a read model that an rmSlice later
// projects into would end up with an EXTRA RM occurrence at the cmdSlice's
// column, stacking the read model directly under the command, which
// violates Event Modeling reading.
// =============================================================================
function placeStoryboardEntries(occs: OccurrenceMap, bySlug: Map<string, Entity>, model: Model, slices: Slice[]): void {
  const slicePositionByKey = new Map<string, number>();
  slices.forEach((slice, col) => {
    slicePositionByKey.set(`${slice.entity.id.namespace}/${slice.entity.id.slug}`, col);
  });
  const tryPlace = (kind: EntityKind, ref: EntityId | undefined, col: number) => {
    if (!ref) return;
    // Honor the "skip if some slice already positioned it" invariant:
    // resolve through `bySlug` directly, then check `occsByEntity`, then
    // let `OccurrenceMap.place` do the actual creation.
    const entity = bySlug.get(slugKey(kind, ref));
    if (!entity) return;
    if (occs.occsByEntity.has(entity.key)) return;
    occs.place(kind, ref, col);
  };
  for (const sb of model.storyboards) {
    const refs = Array.isArray(sb.raw.slices) ? sb.raw.slices : [];
    let firstCol: number | undefined;
    for (const r of refs) {
      const id = (r as { id?: unknown })?.id as { namespace?: string; slug?: string } | undefined;
      if (!id?.namespace || !id?.slug) continue;
      const c = slicePositionByKey.get(`${id.namespace}/${id.slug}`);
      if (c !== undefined) {
        firstCol = c;
        break;
      }
    }
    if (firstCol === undefined) continue;
    const observer = storyboardEntryObserver(sb);
    tryPlace('persona', observer.persona, firstCol);
    tryPlace('ui', observer.ui, firstCol);
    tryPlace('processor', observer.processor, firstCol);
    tryPlace('readModel', storyboardEntryReadModel(sb), firstCol);
  }
}

// =============================================================================
// Pass 6: unwired entities.
//
// Entities no slice references still belong on the board, appended to
// the right so they are visibly "unwired".
// =============================================================================
function placeUnwiredEntities(
  occs: OccurrenceMap,
  bySlug: Map<string, Entity>,
  slices: Slice[],
  laneFor: (entity: Entity) => number,
): void {
  let extraCol = slices.length;
  const laneCursor = new Map<number, number>();
  for (const entity of bySlug.values()) {
    if (occs.occsByEntity.has(entity.key)) continue;
    const lane = laneFor(entity);
    if (lane < 0) continue;
    const col = laneCursor.get(lane) ?? extraCol;
    laneCursor.set(lane, col + 1);
    extraCol = Math.max(extraCol, col + 1);
    occs.createOcc(entity, col);
  }
}

// =============================================================================
// Pass 7: synthesized event → read-model edges.
//
// A projection declared by a ReadModelSlice ties the event to THAT slice's
// own occurrence of the read model (each projecting slice gets its own
// copy of the RM in its column; the tie belongs to the copy that declared
// it). When several slices project into the same read model, targeting
// "the next occurrence in time" instead would pile multiple ties onto the
// first copy and leave the later copies dangling. The forward-in-time
// invariant still holds: when the declaring slice's copy sits at or
// before the event (a storyboard-ordering problem the validator flags as
// PROJECTION_BEFORE_EMITTER), fall back to the next occurrence after the
// event, synthesizing one between columns when the storyboard has none.
// Declaration-only links (RM.source_events with no projecting slice) have
// no owning column and always use the next-occurrence rule.
// =============================================================================
interface RmLink {
  from: Occurrence;
  to: Occurrence;
  animated: boolean;
  ev: EdgeRef;
  slice?: Entity;
}

function resolveRmLinks(
  occs: OccurrenceMap,
  slices: Slice[],
  bySlug: Map<string, Entity>,
): { links: RmLink[]; sliceGaps: Map<number, { events: Set<string>; readModels: Set<string> }> } {
  const rmLinks: { ev: EdgeRef; rm: EntityId; animated: boolean; slice?: Entity; sliceCol?: number }[] = [];
  // (event, read model) pairs already tied by a declaring slice. The
  // RM.source_events pass below only draws pairs no slice covers, so a
  // declared-but-unsliced projection still shows up (that is the
  // RM_EVENT_NOT_PROJECTED situation made visible).
  const sliceCovered = new Set<string>();
  slices.forEach((slice, col) => {
    if (slice.kind !== 'readModelSlice' || !slice.readModel) return;
    for (const ev of slice.events) {
      rmLinks.push({ ev, rm: slice.readModel.id, animated: true, slice: slice.entity, sliceCol: col });
      sliceCovered.add(`${slugKey('event', ev.id)}->${slugKey('readModel', slice.readModel.id)}`);
    }
  });
  for (const entity of bySlug.values()) {
    if (entity.kind !== 'readModel') continue;
    const sources = Array.isArray(entity.raw.sourceEvents) ? entity.raw.sourceEvents : [];
    for (const ref of sources) {
      const id = (ref as { id?: unknown })?.id;
      if (!id) continue;
      const evId = asId(id);
      if (sliceCovered.has(`${slugKey('event', evId)}->${slugKey('readModel', entity.id)}`)) continue;
      rmLinks.push({
        ev: { id: evId, doc: '', metadata: [] },
        rm: entity.id,
        animated: false,
      });
    }
  }

  const linksWithCols = rmLinks
    .map((link) => {
      const evEntity = bySlug.get(slugKey('event', link.ev.id));
      const from = evEntity ? occs.occsByEntity.get(evEntity.key)?.[0] : undefined;
      return from ? { ...link, from } : undefined;
    })
    .filter((x): x is (typeof rmLinks)[number] & { from: Occurrence } => Boolean(x))
    .sort((a, b) => a.from.col - b.from.col);

  // Synthesized columns are modeling gaps: an event updates a read model but
  // no slice in the storyboard shows it afterwards. Track them so the header
  // row can call the gap out instead of leaving a blank.
  const sliceGaps = new Map<number, { events: Set<string>; readModels: Set<string> }>();
  const links: RmLink[] = [];
  for (const link of linksWithCols) {
    const rmEntity = bySlug.get(slugKey('readModel', link.rm));
    if (!rmEntity) continue;
    const knownOccs = [...(occs.occsByEntity.get(rmEntity.key) ?? [])].sort((a, b) => a.col - b.col);
    // Slice-declared projection: prefer the declaring slice's own copy of
    // the read model, as long as it keeps the tie pointing forward.
    let to =
      link.sliceCol !== undefined ? knownOccs.find((o) => o.col === link.sliceCol && o.col > link.from.col) : undefined;
    if (!to) to = knownOccs.find((o) => o.col > link.from.col);
    if (!to) {
      to = occs.createOcc(rmEntity, link.from.col + 0.5);
      if (to) {
        to.synthetic = true;
        const gap = sliceGaps.get(to.col) ?? {
          events: new Set<string>(),
          readModels: new Set<string>(),
        };
        gap.events.add(link.from.entity.title);
        gap.readModels.add(rmEntity.title);
        sliceGaps.set(to.col, gap);
      }
    }
    if (to) {
      links.push({
        from: link.from,
        to,
        animated: link.animated,
        ev: link.ev,
        slice: link.slice,
      });
    }
  }
  return { links, sliceGaps };
}

// =============================================================================
// Pass 8: visual geometry, translating the occurrence map into pixel coords.
// =============================================================================
interface VisualLayout {
  laneY: number[];
  laneDepth: number[];
  colSlots: number[];
  slotOf: Map<number, number>;
  totalCols: number;
  boardH: number;
  boardW: number;
}

function buildVisualLayout(lanes: Lane[], occs: OccurrenceMap, slices: Slice[]): VisualLayout {
  // Lane heights derive from the deepest stack in that lane. stackDepth keys
  // are `${lane}:${col}:${stack}` so we read the third segment to find the
  // deepest vertical occupant in each lane.
  const laneDepth = lanes.map(() => 1);
  for (const sk of occs.stackDepth.keys()) {
    const parts = sk.split(':');
    const lane = Number(parts[0]);
    const stack = Number(parts[2]);
    laneDepth[lane] = Math.max(laneDepth[lane], stack + 1);
  }
  const laneY: number[] = [];
  let y = 0;
  lanes.forEach((_, i) => {
    laneY[i] = y;
    y += laneHeight(laneDepth[i]) + LANE_GAP;
  });
  const boardH = y - LANE_GAP;

  // Columns are time keys (synthesized occurrences live at fractional cols);
  // each distinct key gets its own full-width slot so nothing overlaps.
  //
  // A slice's base integer col becomes a slot only when something occupies
  // it OR the slice is entirely empty (keeping a visible empty column for
  // it, which reads as "something is off"). An automation slice whose
  // sources are reused from earlier columns puts its processor at a
  // sub-column, and reserving the unoccupied base col too would render a
  // full empty column under the slice header.
  const occupiedCols = new Set([...occs.occurrences.values()].map((o) => o.col));
  const baseCols = slices
    .map((_, i) => i)
    .filter((i) => occupiedCols.has(i) || ![...occupiedCols].some((c) => c > i && c < i + 0.4));
  const colSlots = [...new Set([...baseCols, ...occupiedCols])].sort((a, b) => a - b);
  const slotOf = new Map(colSlots.map((c, i) => [c, i]));
  const totalCols = Math.max(colSlots.length, 1);
  const boardW = LANE_LABEL_W + totalCols * COL_W + LANE_PAD;
  return { laneY, laneDepth, colSlots, slotOf, totalCols, boardH, boardW };
}

// =============================================================================
// Header & sticky & edge emission.
// =============================================================================

const HEADER_H = 56;
const HEADER_GAP = 10;
const STORYBOARD_H = 30;
const EVENT_MODEL_H = 28;

function slotSpan(
  colSlots: number[],
  slotOf: Map<number, number>,
  base: number,
): { start: number; end: number } | undefined {
  const slots = colSlots.filter((c) => c >= base && c < base + 0.4).map((c) => slotOf.get(c) ?? 0);
  if (slots.length === 0) return undefined;
  return { start: Math.min(...slots), end: Math.max(...slots) };
}

function snapX(start: number, end: number, totalCols: number): { xStart: number; xEnd: number } {
  return {
    xStart: start === 0 ? -LANE_LABEL_W : start * COL_W + 5,
    xEnd: end === totalCols - 1 ? totalCols * COL_W + LANE_PAD : (end + 1) * COL_W - 5,
  };
}

function buildLaneNodes(lanes: Lane[], laneY: number[], laneDepth: number[], boardW: number): Node[] {
  return lanes.map((lane, i) => ({
    id: `lane:${lane.id}`,
    type: 'lane',
    position: { x: -LANE_LABEL_W, y: laneY[i] },
    data: { label: lane.label, glyph: lane.glyph, tint: lane.tint, entity: lane.entity } satisfies LaneData,
    width: boardW,
    height: laneHeight(laneDepth[i]),
    selectable: false,
    draggable: false,
    zIndex: -10,
  }));
}

function buildSliceAndBandHeaders(
  model: Model,
  slices: Slice[],
  visual: VisualLayout,
  boardH: number,
  staleRefsBySlice: Map<string, number>,
): Node[] {
  const out: Node[] = [];
  slices.forEach((slice, col) => {
    const span = slotSpan(visual.colSlots, visual.slotOf, col);
    if (!span) return;
    const spanW = (span.end - span.start + 1) * COL_W;
    const { xStart, xEnd } = snapX(span.start, span.end, visual.totalCols);
    out.push({
      id: `slice:${slice.entity.key}`,
      type: 'sliceHeader',
      position: { x: xStart, y: -(HEADER_H + HEADER_GAP) },
      data: {
        entity: slice.entity,
        sliceKind: slice.kind,
        status: trackingOf(model, slice.entity)[0]?.status,
        staleRefs: staleRefsBySlice.get(slice.entity.key) || undefined,
      } satisfies SliceHeaderData,
      width: xEnd - xStart,
      height: HEADER_H,
      draggable: false,
    });
    if (col % 2 === 1) {
      out.push({
        id: `band:${col}`,
        type: 'band',
        position: { x: span.start * COL_W, y: 0 },
        data: {},
        width: spanW,
        height: boardH,
        selectable: false,
        draggable: false,
        zIndex: -9,
        style: { pointerEvents: 'none' },
      });
    }
  });
  return out;
}

function buildGapHeaders(
  sliceGaps: Map<number, { events: Set<string>; readModels: Set<string> }>,
  slotOf: Map<number, number>,
): Node[] {
  const out: Node[] = [];
  for (const [col, gap] of sliceGaps) {
    const slot = slotOf.get(col);
    if (slot === undefined) continue;
    out.push({
      id: `gap:${col}`,
      type: 'gapHeader',
      position: { x: slot * COL_W + 5, y: -(HEADER_H + HEADER_GAP) },
      data: {
        events: [...gap.events],
        readModels: [...gap.readModels],
      } satisfies GapHeaderData,
      width: COL_W - 10,
      height: HEADER_H,
      selectable: false,
      draggable: false,
    });
  }
  return out;
}

interface StoryboardHeaderInfo {
  nodes: Node[];
  storyboardSpans: Map<string, { start: number; end: number }>;
  lastColBySb: Map<string, number>;
}

function buildStoryboardHeaders(model: Model, slices: Slice[], visual: VisualLayout): StoryboardHeaderInfo {
  const sliceCol = new Map<string, number>();
  slices.forEach((s, i) => {
    sliceCol.set(`${s.entity.id.namespace}/${s.entity.id.slug}`, i);
  });

  const claimedByStoryboard = new Set<string>();
  const storyboardSpans = new Map<string, { start: number; end: number }>();
  const lastColBySb = new Map<string, number>();
  const nodes: Node[] = [];
  for (const sb of model.storyboards) {
    const refs = Array.isArray(sb.raw.slices) ? sb.raw.slices : [];
    const slots: number[] = [];
    const cols: number[] = [];
    for (const ref of refs) {
      const id = asId((ref as { id?: unknown })?.id);
      const k = `${id.namespace}/${id.slug}`;
      if (claimedByStoryboard.has(k)) continue;
      claimedByStoryboard.add(k);
      const col = sliceCol.get(k);
      const span = col === undefined ? undefined : slotSpan(visual.colSlots, visual.slotOf, col);
      if (span) slots.push(span.start, span.end);
      if (col !== undefined) cols.push(col);
    }
    if (slots.length === 0) continue;
    if (cols.length > 0) lastColBySb.set(sb.key, Math.max(...cols));
    const start = Math.min(...slots);
    const end = Math.max(...slots);
    storyboardSpans.set(`${sb.id.namespace}/${sb.id.slug}`, { start, end });
    const { xStart, xEnd } = snapX(start, end, visual.totalCols);
    const waysIn = model.continuations
      .filter((c) => c.to.key === sb.key)
      .map((c) => ({ from: c.from.title, doc: c.doc }));
    nodes.push({
      id: `storyboard:${sb.key}`,
      type: 'storyboardHeader',
      position: { x: xStart, y: -(HEADER_H + HEADER_GAP + STORYBOARD_H + 8) },
      data: { entity: sb, waysIn } satisfies StickyData,
      width: xEnd - xStart,
      height: STORYBOARD_H,
      draggable: false,
    });
  }
  return { nodes, storyboardSpans, lastColBySb };
}

function buildEventModelHeaders(
  bySlug: Map<string, Entity>,
  storyboardSpans: Map<string, { start: number; end: number }>,
  visual: VisualLayout,
): Node[] {
  const out: Node[] = [];
  for (const em of [...bySlug.values()].filter((e) => e.kind === 'eventModel')) {
    const members = Array.isArray(em.raw.members) ? em.raw.members : [];
    const slots: number[] = [];
    for (const m of members) {
      const member = m as Record<string, unknown>;
      if (member.kind !== 'ENTITY_KIND_STORYBOARD') continue;
      const id = asId(member.id);
      const span = storyboardSpans.get(`${id.namespace}/${id.slug}`);
      if (span) slots.push(span.start, span.end);
    }
    if (slots.length === 0) continue;
    const { xStart, xEnd } = snapX(Math.min(...slots), Math.max(...slots), visual.totalCols);
    out.push({
      id: `eventmodel:${em.key}`,
      type: 'eventModelHeader',
      position: { x: xStart, y: -(HEADER_H + HEADER_GAP + STORYBOARD_H + 8 + EVENT_MODEL_H + 6) },
      data: { entity: em } satisfies StickyData,
      width: xEnd - xStart,
      height: EVENT_MODEL_H,
      draggable: false,
    });
  }
  return out;
}

function buildStickyNodes(
  occs: OccurrenceMap,
  _model: Model,
  slices: Slice[],
  visual: VisualLayout,
  brokenCommands: Set<string>,
  multiIssuerCommands: Set<string>,
  roadsByOcc: Map<string, { title: string; doc: string }[]>,
): Node[] {
  // Instance ordinals: number each occurrence of a type along time so unique
  // instances are tellable apart on the card itself.
  const instanceOf = new Map<string, string>();
  for (const list of occs.occsByEntity.values()) {
    if (list.length < 2) continue;
    const ordered = [...list].sort((a, b) => a.col - b.col);
    ordered.forEach((o, i) => {
      instanceOf.set(o.id, `#${i + 1}/${ordered.length}`);
    });
  }

  const out: Node[] = [];
  for (const occ of occs.occurrences.values()) {
    const moment = occ.synthetic ? undefined : slices[Math.floor(occ.col + 0.0001)]?.entity;
    out.push({
      id: occ.id,
      type: 'sticky',
      position: {
        x: (visual.slotOf.get(occ.col) ?? 0) * COL_W + (COL_W - NODE_W) / 2,
        y: visual.laneY[occ.lane] + LANE_PAD + occ.stack * ROW_H,
      },
      width: NODE_W,
      height: NODE_H,
      data: {
        entity: occ.entity,
        synthetic: occ.synthetic,
        instance: instanceOf.get(occ.id),
        moment,
        instanceUid: instanceUidOf(occ.entity, moment),
        roads: roadsByOcc.get(occ.id),
        noEmittedEvents: occ.entity.kind === 'command' && brokenCommands.has(occ.entity.key),
        multipleIssuers: occ.entity.kind === 'command' && multiIssuerCommands.has(occ.entity.key),
      } satisfies StickyData,
    });
  }
  return out;
}

function buildRoadsByOcc(
  occs: OccurrenceMap,
  model: Model,
  lastColBySb: Map<string, number>,
): Map<string, { title: string; doc: string }[]> {
  const roadsByOcc = new Map<string, { title: string; doc: string }[]>();
  for (const c of model.continuations) {
    const lastCol = lastColBySb.get(c.from.key);
    const entryRm = storyboardEntryReadModel(c.to);
    if (lastCol === undefined || !entryRm) continue;
    const occ = occs.latestBefore('readModel', entryRm, lastCol);
    if (!occ) continue;
    const list = roadsByOcc.get(occ.id) ?? [];
    list.push({ title: c.to.title, doc: c.doc });
    roadsByOcc.set(occ.id, list);
  }
  return roadsByOcc;
}

function buildEdges(occs: OccurrenceMap, _model: Model, slices: Slice[], rmLinks: RmLink[]): Edge[] {
  const edges: Edge[] = [];
  const seenEdges = new Set<string>();

  const OUT: Record<string, string> = {
    ui: 'bs',
    event: 'ts',
    command: 'bs',
    readModel: 'ts',
    processor: 'bs',
  };
  const IN: Record<string, string> = {
    ui: 'bt',
    event: 'tt',
    command: 'tt',
    readModel: 'bt',
    processor: 'bt',
  };

  const connectOcc = (
    a: Occurrence | undefined,
    b: Occurrence | undefined,
    color: string,
    opts: { animated?: boolean; relation?: string; edge?: EdgeRef; slice?: Entity } = {},
  ): void => {
    if (!a || !b || a.id === b.id) return;
    const id = `${a.id}->${b.id}`;
    if (seenEdges.has(id)) return;
    seenEdges.add(id);
    const sourceHandle = OUT[a.entity.kind] ?? 'bs';
    const targetHandle = IN[b.entity.kind] ?? 'tt';
    edges.push({
      id,
      source: a.id,
      target: b.id,
      sourceHandle,
      targetHandle,
      animated: opts.animated ?? false,
      data: {
        relation: opts.relation ?? '',
        doc: opts.edge?.doc ?? '',
        metadata: opts.edge?.metadata ?? [],
        source: a.entity,
        target: b.entity,
        slice: opts.slice,
      } satisfies BoardEdgeData,
      style: { stroke: color, strokeWidth: 1.8 },
    });
  };

  // Edge colors follow the classic Event Modeling convention: an arrow
  // carries the color of the payload flowing along it. Command flow
  // (UI or processor into a command) is sky like the command card,
  // event data (command emits, projections into read models) is orange
  // like the event card, and read model data (into a UI or a processor)
  // is emerald like the read model card. Cross-context projections keep
  // the distinct boundary color below.
  slices.forEach((slice, col) => {
    if (slice.kind === 'commandSlice') {
      connectOcc(occs.at('ui', slice.ui?.id, col), occs.at('command', slice.command?.id), '#0ea5e9', {
        animated: true,
        relation: 'triggers',
        edge: slice.command,
        slice: slice.entity,
      });
      for (const ev of slice.events)
        connectOcc(occs.at('command', slice.command?.id), occs.at('event', ev.id), '#f97316', {
          animated: true,
          relation: 'emits',
          edge: ev,
          slice: slice.entity,
        });
    }
    if (slice.kind === 'uiSlice') {
      for (const rm of slice.readModels)
        connectOcc(occs.latestBefore('readModel', rm.id, col), occs.at('ui', slice.ui?.id, col), '#10b981', {
          animated: true,
          relation: 'renders into',
          edge: rm,
          slice: slice.entity,
        });
    }
    if (slice.kind === 'automationSlice') {
      for (const rm of slice.readModels)
        connectOcc(
          occs.latestBefore('readModel', rm.id, col),
          occs.at('processor', slice.processor?.id, col),
          '#10b981',
          {
            animated: true,
            relation: 'observed by',
            edge: rm,
            slice: slice.entity,
          },
        );
      connectOcc(occs.at('processor', slice.processor?.id, col), occs.at('command', slice.command?.id), '#0ea5e9', {
        animated: true,
        relation: 'issues',
        edge: slice.command,
        slice: slice.entity,
      });
    }
  });

  for (const link of rmLinks) {
    // A projection crossing a namespace is the context seam: an upstream
    // event translated into this context's subscription view. Boundary
    // color, "feeds context": pub/sub translation, not a local update.
    const cross = link.from && link.to && link.from.entity.id.namespace !== link.to.entity.id.namespace;
    connectOcc(link.from, link.to, cross ? '#d946ef' : '#f97316', {
      animated: link.animated,
      relation: cross ? 'feeds context' : 'updates',
      edge: link.ev,
      slice: link.slice,
    });
  }

  return edges;
}

// =============================================================================
// Orchestrator.
// =============================================================================
export function layoutBoard(model: Model): { nodes: Node[]; edges: Edge[] } {
  const bySlug = buildLatestBySlug(model);
  const { lanes, laneFor } = buildLaneMap(model, bySlug);
  const slices = orderedSlices(model);
  const occs = new OccurrenceMap(bySlug, laneFor);

  placeSlicePrimaries(occs, slices);
  placeConsumerSources(occs, slices);
  placeReferencedFallbacks(occs, slices);
  placeStoryboardEntries(occs, bySlug, model, slices);
  placeUnwiredEntities(occs, bySlug, slices, laneFor);

  const { links: rmLinks, sliceGaps } = resolveRmLinks(occs, slices, bySlug);
  const visual = buildVisualLayout(lanes, occs, slices);

  const staleRefsBySlice = new Map<string, number>();
  for (const slice of slices) {
    const count = staleRefsOf(model, slice.entity).length;
    if (count > 0) staleRefsBySlice.set(slice.entity.key, count);
  }

  const laneNodes = buildLaneNodes(lanes, visual.laneY, visual.laneDepth, visual.boardW);
  const sliceNodes = buildSliceAndBandHeaders(model, slices, visual, visual.boardH, staleRefsBySlice);
  const gapNodes = buildGapHeaders(sliceGaps, visual.slotOf);
  const sbHeaders = buildStoryboardHeaders(model, slices, visual);
  const emNodes = buildEventModelHeaders(bySlug, sbHeaders.storyboardSpans, visual);
  const roadsByOcc = buildRoadsByOcc(occs, model, sbHeaders.lastColBySb);

  const brokenCommands = commandsWithoutEmittedEvents(model);
  const multiIssuerCommands = commandsWithMultipleIssuers(model);
  const stickyNodes = buildStickyNodes(occs, model, slices, visual, brokenCommands, multiIssuerCommands, roadsByOcc);
  const edges = buildEdges(occs, model, slices, rmLinks);

  return {
    nodes: [...laneNodes, ...sliceNodes, ...gapNodes, ...sbHeaders.nodes, ...emNodes, ...stickyNodes],
    edges,
  };
}

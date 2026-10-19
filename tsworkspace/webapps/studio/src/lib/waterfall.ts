// Waterfall view: the model read as rows of steps against the slice
// timeline. The header numbers the slices in `orderedSlices` order, and each
// row marks the slice where its step happens, so a chain reads top to bottom
// and steps right as time moves forward.
//
// The same steps group two ways:
//   * By storyboard. Each storyboard is one chain, walked in its own slice
//     order. The first chain that reaches an entity owns it and shows it as
//     a step. A later chain that reaches it shows a compact join row that
//     points to the owner, and a repeat occurrence inside one chain shows an
//     "above" row, so recurrence stays visible without repeating the step.
//     Entities no storyboard reaches close the list, ordered by the
//     happens-next links.
//   * By object type. One group per Board lane and kind, so the rows sit
//     where the Board puts their cards. Cables between groups count the
//     happens-next links that join them.
//
// Steps are the five kinds the Board draws as cards. Slice refs resolve to
// the latest loaded version, as on the Board.

import { boardLanes, FLOW_COLOR } from './layout';
import {
  asId,
  type Entity,
  type EntityKind,
  type Model,
  orderedSlices,
  partitionByNamespace,
  type SliceView,
  slugKey,
  versionOf,
} from './model';

/** The Board's card kinds, in Event Modeling flow order. */
export const STEP_KINDS = ['ui', 'command', 'event', 'readModel', 'processor'] as const;
export type StepKind = (typeof STEP_KINDS)[number];

const KIND_ORDER = new Map<EntityKind, number>(STEP_KINDS.map((k, i) => [k, i]));

/** Group titles in the object type grouping. The lane column names the lane. */
export const STEP_KIND_PLURAL: Record<StepKind, string> = {
  ui: 'UIs',
  command: 'Commands',
  event: 'Events',
  readModel: 'Read models',
  processor: 'Automations',
};

/** A happens-next link between two step entities, at the type level. */
export interface FlowLink {
  from: Entity;
  to: Entity;
  /** The Board's edge relation, e.g. "triggers" or "renders into". */
  relation: string;
  /** The slice that declares the link. Absent for a projection declared only in `source_events`. */
  slice?: Entity;
}

/** Code references bundled by repository and the first two folders of their path. */
export interface CodeBundle {
  key: string;
  repo: string;
  folder: string;
  label: string;
  refs: number;
  files: string[];
}

export interface RowTarget {
  group: string;
  row: string;
}

export interface WaterfallRow {
  key: string;
  // step: the entity's home row in this grouping. join: the entity's home
  // is in another group. above: a repeat occurrence of an entity that this
  // group already shows.
  type: 'step' | 'join' | 'above';
  entity: Entity;
  lane: string;
  depth: number;
  /** Slice positions this row marks, 0 based. Empty when no slice references the entity. */
  cols: number[];
  /** How long the occurrence stays relevant: up to the last slice that still reads it. */
  span?: { start: number; end: number };
  /** join and above rows: the row this one points to. */
  target?: RowTarget;
  /** join rows: the title of the group that owns the entity. */
  targetTitle?: string;
  /** Step rows: titles of the other groups that join here. */
  joinedBy: string[];
  linksIn: number;
  linksOut: number;
  bundles: CodeBundle[];
}

export interface ColumnSummary {
  col: number;
  total: number;
  kinds: { kind: EntityKind; count: number }[];
}

export interface WaterfallGroup {
  key: string;
  title: string;
  /** Storyboard chains: the storyboard itself, so the header can select it. */
  entity?: Entity;
  /** Object type groups: the kind and the Board lane they hold. */
  kind?: StepKind;
  lane?: string;
  rows: WaterfallRow[];
  /** Entities whose home row is in this group. */
  members: Entity[];
  columns: ColumnSummary[];
  span?: { start: number; end: number };
  linksIn: number;
  linksOut: number;
  bundles: CodeBundle[];
}

/** Every happens-next link from one object type group to another. */
export interface WaterfallCable {
  key: string;
  from: string;
  to: string;
  links: FlowLink[];
  color: string;
}

export interface Waterfall {
  /** Header columns, in `orderedSlices` order. */
  slices: SliceView[];
  links: FlowLink[];
  storyboards: WaterfallGroup[];
  /** Entity key to its home row in the storyboard grouping. */
  storyboardHome: Map<string, RowTarget>;
  types: WaterfallGroup[];
  /** Entity key to its home row in the object type grouping. */
  typeHome: Map<string, RowTarget>;
  cables: WaterfallCable[];
  /** Entity key to the slice positions where it occurs. */
  occurrences: Map<string, number[]>;
  /** Steps that more than one group reaches in the storyboard grouping. */
  shared: number;
  /** False when no entity in the model carries a code reference, so the code column has nothing to show. */
  hasCode: boolean;
}

export const NOT_IN_A_STORYBOARD = 'Not in a storyboard';
const UNASSIGNED_GROUP = 'reserved:unassigned';

const nsSlug = (id: { namespace: string; slug: string }) => `${id.namespace}/${id.slug}`;

function latestBySlug(model: Model): Map<string, Entity> {
  const bySlug = new Map<string, Entity>();
  for (const e of model.entities) {
    const k = slugKey(e.kind, e.id);
    const cur = bySlug.get(k);
    if (!cur || versionOf(e.id) > versionOf(cur.id)) bySlug.set(k, e);
  }
  return bySlug;
}

type Resolve = (kind: StepKind, ref: { id: { namespace: string; slug: string } } | undefined) => Entity | undefined;

function resolver(bySlug: Map<string, Entity>): Resolve {
  return (kind, ref) => (ref ? bySlug.get(slugKey(kind, ref.id)) : undefined);
}

const present = (list: (Entity | undefined)[]): Entity[] => list.filter((e): e is Entity => Boolean(e));

// What a slice reads (inputs) and what happens in it (primaries). Inputs
// already happened earlier in time, so a chain shows them only when nothing
// before introduced them.
function stepsOf(slice: SliceView, resolve: Resolve): { inputs: Entity[]; primaries: Entity[] } {
  switch (slice.kind) {
    case 'commandSlice':
      return {
        inputs: [],
        primaries: present([
          resolve('ui', slice.ui),
          resolve('command', slice.command),
          ...slice.events.map((e) => resolve('event', e)),
        ]),
      };
    case 'readModelSlice':
      return {
        inputs: present(slice.events.map((e) => resolve('event', e))),
        primaries: present([resolve('readModel', slice.readModel)]),
      };
    case 'automationSlice':
      return {
        inputs: present(slice.readModels.map((r) => resolve('readModel', r))),
        primaries: present([resolve('processor', slice.processor), resolve('command', slice.command)]),
      };
    case 'uiSlice':
      return {
        inputs: present(slice.readModels.map((r) => resolve('readModel', r))),
        primaries: present([resolve('ui', slice.ui)]),
      };
  }
}

/**
 * The happens-next links the Board and Sequence views draw, at the type
 * level: ui to command, command to event, event to read model, read model
 * to processor, processor to command, and read model to ui. A read model's
 * `source_events` with no projecting slice still links, as on the Board.
 * Each pair of entities links once.
 */
export function happensNextLinks(model: Model): FlowLink[] {
  const bySlug = latestBySlug(model);
  const resolve = resolver(bySlug);
  const links: FlowLink[] = [];
  const seen = new Set<string>();
  const add = (from: Entity | undefined, to: Entity | undefined, relation: string, slice?: Entity) => {
    if (!from || !to || from.key === to.key) return;
    const k = `${from.key}->${to.key}`;
    if (seen.has(k)) return;
    seen.add(k);
    links.push({ from, to, relation, slice });
  };
  const projection = (ev: Entity, rm: Entity) => (ev.id.namespace !== rm.id.namespace ? 'feeds context' : 'updates');

  for (const s of orderedSlices(model)) {
    if (s.kind === 'commandSlice') {
      const cmd = resolve('command', s.command);
      add(resolve('ui', s.ui), cmd, 'triggers', s.entity);
      for (const e of s.events) add(cmd, resolve('event', e), 'emits', s.entity);
    } else if (s.kind === 'readModelSlice') {
      const rm = resolve('readModel', s.readModel);
      for (const e of s.events) {
        const ev = resolve('event', e);
        if (ev && rm) add(ev, rm, projection(ev, rm), s.entity);
      }
    } else if (s.kind === 'automationSlice') {
      const proc = resolve('processor', s.processor);
      for (const r of s.readModels) add(resolve('readModel', r), proc, 'observed by', s.entity);
      add(proc, resolve('command', s.command), 'issues', s.entity);
    } else {
      const ui = resolve('ui', s.ui);
      for (const r of s.readModels) add(resolve('readModel', r), ui, 'renders into', s.entity);
    }
  }
  for (const rm of bySlug.values()) {
    if (rm.kind !== 'readModel') continue;
    for (const ref of Array.isArray(rm.raw.sourceEvents) ? rm.raw.sourceEvents : []) {
      const ev = bySlug.get(slugKey('event', asId((ref as Record<string, unknown>)?.id)));
      if (ev) add(ev, rm, projection(ev, rm));
    }
  }
  return links;
}

/** The color the Board gives an edge with this relation. */
export function flowColor(relation: string): string {
  switch (relation) {
    case 'triggers':
    case 'issues':
      return FLOW_COLOR.command;
    case 'renders into':
    case 'observed by':
      return FLOW_COLOR.readModel;
    case 'feeds context':
      return FLOW_COLOR.seam;
    default:
      return FLOW_COLOR.event;
  }
}

// The first two folders name the part of a repository that implements
// something; deeper folders split one feature into chips nobody can scan.
function folderOf(path: string): string {
  return path.split('/').slice(0, -1).slice(0, 2).join('/');
}

/** Bundles every complete `CodeRefAnnotation` on the entities, most referenced first. */
export function codeBundles(entities: Entity[]): CodeBundle[] {
  const byKey = new Map<string, { bundle: CodeBundle; files: Set<string> }>();
  for (const entity of entities) {
    for (const a of entity.annotations) {
      if (a.type !== 'CodeRefAnnotation') continue;
      const repo = String(a.data.repo ?? '');
      const path = String(a.data.path ?? '');
      // An incomplete reference cannot be placed in a folder. The Inspector
      // shows it with its CODE_REF_INCOMPLETE explanation.
      if (!repo || !path) continue;
      const folder = folderOf(path);
      const key = `${repo}:${folder}`;
      let entry = byKey.get(key);
      if (!entry) {
        entry = {
          bundle: { key, repo, folder, label: folder ? `${repo}/${folder}` : repo, refs: 0, files: [] },
          files: new Set<string>(),
        };
        byKey.set(key, entry);
      }
      entry.bundle.refs += 1;
      entry.files.add(path);
    }
  }
  return [...byKey.values()]
    .map(({ bundle, files }) => ({ ...bundle, files: [...files].sort() }))
    .sort((a, b) => b.refs - a.refs || a.label.localeCompare(b.label));
}

function summarize(rows: { entity: Entity; cols: number[] }[]): ColumnSummary[] {
  const byCol = new Map<number, Map<EntityKind, number>>();
  for (const r of rows) {
    for (const col of r.cols) {
      const kinds = byCol.get(col) ?? new Map<EntityKind, number>();
      kinds.set(r.entity.kind, (kinds.get(r.entity.kind) ?? 0) + 1);
      byCol.set(col, kinds);
    }
  }
  return [...byCol.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([col, kinds]) => {
      const list = [...kinds.entries()]
        .map(([kind, count]) => ({ kind, count }))
        .sort((a, b) => (KIND_ORDER.get(a.kind) ?? 9) - (KIND_ORDER.get(b.kind) ?? 9));
      return { col, total: list.reduce((n, k) => n + k.count, 0), kinds: list };
    });
}

function spanOf(cols: number[]): { start: number; end: number } | undefined {
  if (cols.length === 0) return undefined;
  return { start: Math.min(...cols), end: Math.max(...cols) };
}

export function buildWaterfall(model: Model): Waterfall {
  const bySlug = latestBySlug(model);
  const resolve = resolver(bySlug);
  const slices = orderedSlices(model);
  const sliceBySlug = new Map(slices.map((s) => [nsSlug(s.entity.id), s]));
  const posOf = new Map(slices.map((s, i) => [nsSlug(s.entity.id), i]));
  const steps = [...bySlug.values()].filter((e) => KIND_ORDER.has(e.kind));
  const links = happensNextLinks(model);

  // Where each entity happens (primary) and every slice that touches it at
  // all (reads included). Positions arrive in ascending order.
  const primary = new Map<string, number[]>();
  const touched = new Map<string, number[]>();
  const mark = (map: Map<string, number[]>, e: Entity, col: number) => {
    const list = map.get(e.key) ?? [];
    if (list[list.length - 1] !== col) list.push(col);
    map.set(e.key, list);
  };
  slices.forEach((slice, col) => {
    const { inputs, primaries } = stepsOf(slice, resolve);
    for (const e of primaries) {
      mark(primary, e, col);
      mark(touched, e, col);
    }
    for (const e of inputs) mark(touched, e, col);
  });
  // As on the Board, an entity only ever read (an event from outside, a
  // read model nothing projects) occurs where it is first read, and a read
  // model read before its first projection occurs at that read too.
  const occurrences = new Map<string, number[]>();
  for (const e of steps) {
    const prim = primary.get(e.key) ?? [];
    const firstTouch = touched.get(e.key)?.[0];
    let cols = prim;
    if (firstTouch !== undefined && (prim.length === 0 || (e.kind === 'readModel' && firstTouch < prim[0]))) {
      cols = [firstTouch, ...prim];
    }
    if (cols.length > 0) occurrences.set(e.key, cols);
  }
  const firstCol = (e: Entity) => occurrences.get(e.key)?.[0] ?? Number.MAX_SAFE_INTEGER;
  const byTimeline = (a: Entity, b: Entity) =>
    firstCol(a) - firstCol(b) ||
    (KIND_ORDER.get(a.kind) ?? 9) - (KIND_ORDER.get(b.kind) ?? 9) ||
    a.title.localeCompare(b.title);

  // An occurrence stays relevant until the last slice that reads it before
  // the entity happens again: an event until its last projection, a read
  // model state until its last reader before the next update.
  const spanFrom = (e: Entity, col: number) => {
    const next = (primary.get(e.key) ?? []).find((p) => p > col);
    const reads = (touched.get(e.key) ?? []).filter((p) => p >= col && (next === undefined || p < next));
    return { start: col, end: Math.max(col, ...reads) };
  };

  const linksOut = new Map<string, number>();
  const linksIn = new Map<string, number>();
  const outgoing = new Map<string, FlowLink[]>();
  for (const l of links) {
    linksOut.set(l.from.key, (linksOut.get(l.from.key) ?? 0) + 1);
    linksIn.set(l.to.key, (linksIn.get(l.to.key) ?? 0) + 1);
    const list = outgoing.get(l.from.key) ?? [];
    list.push(l);
    outgoing.set(l.from.key, list);
  }
  for (const list of outgoing.values()) list.sort((a, b) => byTimeline(a.to, b.to));

  // The Board lays each namespace out on its own, so lanes come from each
  // namespace's own lane map. Several namespaces on screen name their lanes
  // with the namespace, because every namespace has its own Timeline.
  const laneOf = new Map<string, { key: string; label: string; order: number }>();
  const parts = partitionByNamespace(model);
  let laneOrder = 0;
  for (const part of parts) {
    const { lanes, laneFor } = boardLanes(part);
    const ns = part.namespaces[0] ?? '';
    const base = laneOrder;
    for (const e of steps) {
      if (e.id.namespace !== ns && parts.length > 1) continue;
      const idx = laneFor(e);
      const lane = lanes[idx];
      if (!lane) continue;
      laneOf.set(e.key, {
        key: parts.length > 1 ? `${ns}|${lane.id}` : lane.id,
        label: parts.length > 1 ? `${ns} · ${lane.label}` : lane.label,
        order: base + idx,
      });
    }
    laneOrder += lanes.length;
  }
  const laneLabel = (e: Entity) => laneOf.get(e.key)?.label ?? '';

  const row = (
    group: string,
    index: number,
    type: WaterfallRow['type'],
    entity: Entity,
    cols: number[],
    depth: number,
  ): WaterfallRow => ({
    key: `${group}#${index}`,
    type,
    entity,
    lane: laneLabel(entity),
    depth,
    cols,
    joinedBy: [],
    linksIn: linksIn.get(entity.key) ?? 0,
    linksOut: linksOut.get(entity.key) ?? 0,
    bundles: type === 'step' ? codeBundles([entity]) : [],
  });

  // ---- by storyboard ----------------------------------------------------------
  const storyboardHome = new Map<string, RowTarget>();
  const ownerTitle = new Map<string, string>();
  const rowByKey = new Map<string, WaterfallRow>();
  const storyboards: WaterfallGroup[] = [];
  const inStoryboard = new Set<string>();

  const finishGroup = (
    key: string,
    title: string,
    rows: WaterfallRow[],
    extra: Entity[],
    entity?: Entity,
  ): WaterfallGroup => {
    const members = rows.filter((r) => r.type === 'step').map((r) => r.entity);
    return {
      key,
      title,
      entity,
      rows,
      members,
      columns: summarize(rows),
      span: spanOf(rows.flatMap((r) => r.cols)),
      linksIn: 0,
      linksOut: 0,
      bundles: codeBundles([...members, ...extra]),
    };
  };

  for (const sb of model.storyboards) {
    const group = `storyboard:${slugKey('storyboard', sb.id)}`;
    const rows: WaterfallRow[] = [];
    // Entity key to the first row this chain gave it, whatever its type.
    const firstRow = new Map<string, string>();
    const chainSlices: Entity[] = [];
    const place = (e: Entity, col: number) => {
      const mine = firstRow.get(e.key);
      const owner = storyboardHome.get(e.key);
      let r: WaterfallRow;
      if (mine) {
        r = row(group, rows.length, 'above', e, [col], 0);
        r.target = { group, row: mine };
      } else if (!owner) {
        r = row(group, rows.length, 'step', e, [col], 0);
        r.span = spanFrom(e, col);
        storyboardHome.set(e.key, { group, row: r.key });
        ownerTitle.set(e.key, sb.title);
        firstRow.set(e.key, r.key);
      } else {
        r = row(group, rows.length, 'join', e, [col], 0);
        r.target = owner;
        r.targetTitle = ownerTitle.get(e.key);
        firstRow.set(e.key, r.key);
        const home = rowByKey.get(owner.row);
        if (home && !home.joinedBy.includes(sb.title)) home.joinedBy.push(sb.title);
      }
      rows.push(r);
      rowByKey.set(r.key, r);
    };
    let prev: SliceView | undefined;
    for (const ref of Array.isArray(sb.raw.slices) ? sb.raw.slices : []) {
      const k = nsSlug(asId((ref as Record<string, unknown>)?.id));
      const slice = sliceBySlug.get(k);
      const col = posOf.get(k);
      if (!slice || col === undefined) continue;
      chainSlices.push(slice.entity);
      inStoryboard.add(slice.entity.key);
      const { inputs, primaries } = stepsOf(slice, resolve);
      // An input this chain already shows is just being read again.
      for (const e of inputs) if (!firstRow.has(e.key)) place(e, col);
      // A command slice with no screen that handles the command the
      // automation right before it issued is the same moment, as in the
      // Sequence view, so it does not repeat the command.
      const issued = prev?.kind === 'automationSlice' && !slice.ui ? resolve('command', prev.command) : undefined;
      for (const e of primaries) {
        if (slice.kind === 'commandSlice' && issued?.key === e.key) continue;
        place(e, col);
      }
      prev = slice;
    }
    storyboards.push(finishGroup(group, sb.title, rows, chainSlices, sb));
  }

  // Entities no storyboard reaches, depth first along happens-next links,
  // starting from each entity that nothing else in the set leads into.
  const rest = steps.filter((e) => !storyboardHome.has(e.key)).sort(byTimeline);
  if (rest.length > 0) {
    const inRest = new Set(rest.map((e) => e.key));
    const hasIn = new Set(links.filter((l) => inRest.has(l.from.key)).map((l) => l.to.key));
    const rows: WaterfallRow[] = [];
    const visited = new Map<string, string>();
    // A row reached along a link that points elsewhere marks the slice that
    // declares the link: that is where this group touches the entity again.
    const walk = (e: Entity, depth: number, via?: FlowLink) => {
      const cols = occurrences.get(e.key)?.slice(0, 1) ?? [];
      const viaCol = via?.slice ? posOf.get(nsSlug(via.slice.id)) : undefined;
      const pointerCols = viaCol === undefined ? cols : [viaCol];
      const seen = visited.get(e.key);
      if (seen) {
        const r = row(UNASSIGNED_GROUP, rows.length, 'above', e, pointerCols, depth);
        r.target = { group: UNASSIGNED_GROUP, row: seen };
        rows.push(r);
        return;
      }
      const owner = storyboardHome.get(e.key);
      if (owner) {
        const r = row(UNASSIGNED_GROUP, rows.length, 'join', e, pointerCols, depth);
        r.target = owner;
        r.targetTitle = ownerTitle.get(e.key);
        const home = rowByKey.get(owner.row);
        if (home && !home.joinedBy.includes(NOT_IN_A_STORYBOARD)) home.joinedBy.push(NOT_IN_A_STORYBOARD);
        rows.push(r);
        return;
      }
      const r = row(UNASSIGNED_GROUP, rows.length, 'step', e, cols, depth);
      if (cols.length > 0) r.span = spanFrom(e, cols[0]);
      rows.push(r);
      visited.set(e.key, r.key);
      storyboardHome.set(e.key, { group: UNASSIGNED_GROUP, row: r.key });
      for (const l of outgoing.get(e.key) ?? []) walk(l.to, depth + 1, l);
    };
    for (const e of rest) if (!hasIn.has(e.key) && !visited.has(e.key)) walk(e, 0);
    // A loop with no way in has no root: start it at its earliest step.
    for (const e of rest) if (!visited.has(e.key)) walk(e, 0);
    const looseSlices = slices.filter((s) => !inStoryboard.has(s.entity.key)).map((s) => s.entity);
    storyboards.push(finishGroup(UNASSIGNED_GROUP, NOT_IN_A_STORYBOARD, rows, looseSlices));
  }

  // ---- by object type ---------------------------------------------------------
  const typeHome = new Map<string, RowTarget>();
  const typeGroups = new Map<string, WaterfallGroup>();
  const groupOrder = new Map<string, number>();
  for (const e of [...steps].sort(byTimeline)) {
    const lane = laneOf.get(e.key) ?? { key: 'reserved:none', label: '', order: Number.MAX_SAFE_INTEGER };
    const kind = e.kind as StepKind;
    const key = `type:${lane.key}|${kind}`;
    let g = typeGroups.get(key);
    if (!g) {
      g = {
        key,
        title: STEP_KIND_PLURAL[kind],
        kind,
        lane: lane.label,
        rows: [],
        members: [],
        columns: [],
        linksIn: 0,
        linksOut: 0,
        bundles: [],
      };
      typeGroups.set(key, g);
      groupOrder.set(key, lane.order * STEP_KINDS.length + (KIND_ORDER.get(kind) ?? 0));
    }
    const r = row(key, g.rows.length, 'step', e, occurrences.get(e.key) ?? [], 1);
    r.span = spanOf([...(occurrences.get(e.key) ?? []), ...(touched.get(e.key) ?? [])]);
    g.rows.push(r);
    g.members.push(e);
    typeHome.set(e.key, { group: key, row: r.key });
  }
  const types = [...typeGroups.values()].sort((a, b) => (groupOrder.get(a.key) ?? 0) - (groupOrder.get(b.key) ?? 0));
  // Events sit in one lane per stream, so one kind can fill several groups.
  // The lane then goes into the title, or the reader sees three "Events".
  const groupsPerKind = new Map<string, number>();
  for (const g of types) groupsPerKind.set(g.title, (groupsPerKind.get(g.title) ?? 0) + 1);
  for (const g of types) {
    if ((groupsPerKind.get(g.title) ?? 0) > 1 && g.lane) g.title = `${g.title} · ${g.lane}`;
    g.columns = summarize(g.rows);
    g.span = spanOf(g.rows.flatMap((r) => r.cols));
    g.bundles = codeBundles(g.members);
  }

  const cableByKey = new Map<string, WaterfallCable>();
  for (const l of links) {
    const from = typeHome.get(l.from.key)?.group;
    const to = typeHome.get(l.to.key)?.group;
    if (!from || !to) continue;
    const key = `${from}>${to}`;
    let c = cableByKey.get(key);
    if (!c) {
      c = { key, from, to, links: [], color: flowColor(l.relation) };
      cableByKey.set(key, c);
    }
    c.links.push(l);
  }
  const cables = [...cableByKey.values()].sort((a, b) => b.links.length - a.links.length || a.key.localeCompare(b.key));
  for (const c of cables) {
    // A cable keeps the color of the flow it carries. Only a cable made
    // entirely of cross-context projections takes the boundary color.
    const local = c.links.find((l) => l.relation !== 'feeds context');
    c.color = flowColor((local ?? c.links[0]).relation);
    const from = typeGroups.get(c.from);
    const to = typeGroups.get(c.to);
    if (from) from.linksOut += c.links.length;
    if (to) to.linksIn += c.links.length;
  }

  return {
    slices,
    links,
    storyboards,
    storyboardHome,
    types,
    typeHome,
    cables,
    occurrences,
    shared: [...rowByKey.values()].filter((r) => r.joinedBy.length > 0).length,
    hasCode: model.entities.some((e) => e.annotations.some((a) => a.type === 'CodeRefAnnotation')),
  };
}

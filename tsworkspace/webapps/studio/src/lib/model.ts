// Normalization of the gRPC bridge's wire JSON (proto-loader camelCase,
// no virtual oneof discriminators: the envelope kind is whichever member
// key is present) into flat view models the canvas and inspector consume.

export interface EntityId {
  namespace: string;
  slug: string;
  // Wire shape: the bridge serializes proto uint64 with `longs: String`,
  // so this comes through as a string. Always coerce via `versionOf()`
  // before comparing numerically; never use `>` directly on the field.
  version: string;
}

/**
 * Coerces an `EntityId.version` (string on the wire) to a number for
 * comparison or arithmetic. Returns `0` for unparseable inputs so a
 * malformed version sorts before any real value.
 */
export function versionOf(id: { version: string } | undefined | null): number {
  if (!id) return 0;
  const n = Number(id.version);
  return Number.isFinite(n) ? n : 0;
}

export type EntityKind =
  | 'event'
  | 'command'
  | 'readModel'
  | 'processor'
  | 'ui'
  | 'persona'
  | 'swimlane'
  | 'commandSlice'
  | 'readModelSlice'
  | 'automationSlice'
  | 'uiSlice'
  | 'storyboard'
  | 'eventModel'
  | 'component'
  | 'externalSystem'
  | 'tracker'
  | 'boundedContext'
  | 'domain'
  | 'subdomain'
  | 'schema'
  | 'project'
  | 'screen'
  | 'term'
  | 'ambiguity'
  | 'serviceLevelIndicator'
  | 'serviceLevelObjective'
  | 'alertPolicy'
  | 'alertNotificationTarget'
  | 'typeLibrary';

export interface FieldTypeConfig {
  version?: number;
  values?: string[];
  fields?: FieldSpec[];
  ref?: unknown;
}

export interface FieldSpec {
  name?: string;
  type?: Record<string, FieldTypeConfig>;
  doc?: string;
  [key: string]: unknown;
}

export function fieldTypeLabel(f: FieldSpec): string | undefined {
  const kind = Object.keys(f.type ?? {})[0];
  if (!kind) return undefined;
  const cfg = (f.type as Record<string, FieldTypeConfig>)[kind] ?? {};
  if (kind === 'uuid' && cfg.version) return `uuid v${cfg.version}`;
  if (kind === 'enumeration') return `enum(${(cfg.values ?? []).join(' | ')})`;
  if (kind === 'object') return `object{${(cfg.fields ?? []).map((x) => x.name).join(', ')}}`;
  return kind;
}

// A decoded canonical annotation from the entity's metadata (Any) list,
// e.g. type "ComponentAnnotation" with data { name, doc }.
export interface Annotation {
  type: string;
  data: Record<string, unknown>;
}

function decodeAnnotations(metadata: unknown): Annotation[] {
  return (Array.isArray(metadata) ? metadata : []).flatMap((m) => {
    if (!m || typeof m !== 'object') return [];
    const t = (m as Record<string, unknown>)['@type'];
    if (typeof t !== 'string') return [];
    const { '@type': _ignored, ...data } = m as Record<string, unknown>;
    return [{ type: t.split('.').pop() ?? t, data }];
  });
}

export interface Entity {
  kind: EntityKind;
  id: EntityId;
  key: string;
  title: string;
  doc: string;
  role?: string;
  order?: number;
  color?: string;
  swimlane?: EntityId;
  // Read models projected from outside the system (proto ExternalSource):
  // populated by an external system, not by internal events.
  externalSource?: { system: string; descriptor: string };
  // Components and external systems: technologies relied on (kafka, nats, ...).
  tech?: string[];
  // Processors: the external systems this processor calls (outbound seam).
  calls?: EntityId[];
  // Swimlanes only: the runtime stream identity, e.g. "order-{order_id}".
  streamId?: string;
  // UIs only: the Screen slot this moment contributes to.
  screenSlot?: { screen: EntityId; slot: string };
  // System-owned incarnation identity (Kubernetes metadata.uid style):
  // assigned on create, stable across updates, new on delete+recreate.
  uid?: string;
  createdAt?: string;
  annotations: Annotation[];
  fields: FieldSpec[];
  raw: Record<string, unknown>;
}

// A slice endpoint is a typed Edge in the proto: a pointer plus design data
// attached to the connection itself (doc, Any annotations).
export interface EdgeRef {
  id: EntityId;
  doc: string;
  metadata: unknown[];
}

// One concrete example in a scenario: a typed ref plus its example data
// (pre-codegen convention: payloads are Any-packed google.protobuf.Struct).
export interface ScenarioExample {
  kind: 'event' | 'command' | 'readModel';
  id: EntityId;
  data?: Record<string, unknown>;
}

export interface ScenarioView {
  id: string;
  title: string;
  doc: string;
  given: ScenarioExample[];
  whenDoc?: string;
  when: ScenarioExample[];
  thenEmits: ScenarioExample[];
  thenState?: ScenarioExample;
  thenCommand?: ScenarioExample;
  reject?: { reasonCode: string; doc: string };
  annotations: Annotation[];
}

export interface SliceView {
  kind: 'commandSlice' | 'readModelSlice' | 'automationSlice' | 'uiSlice';
  entity: Entity;
  persona?: EdgeRef;
  ui?: EdgeRef;
  command?: EdgeRef;
  events: EdgeRef[];
  readModel?: EdgeRef;
  readModels: EdgeRef[];
  processor?: EdgeRef;
  scenarios: ScenarioView[];
}

// A derived fork in the road: the `from` storyboard's outcome event
// (`afterEvent`) has the `to` storyboard's branch event as a legal successor
// in the swimlane's transition table. The guard (`doc`) is the condition.
// Branches are exclusive alternatives; cycles are legal (lifecycles loop).
// Migration-pending detection: a slice ref that pins a version older than
// the latest stored version means a breaking change is mid-flight. Derived
// by comparing pinned refs to the loaded (latest) entities; nobody
// maintains it.
export interface StaleRef {
  label: string;
  ref: EntityId;
  latest: string;
}

export function staleRefsOf(model: Model, entity: Entity): StaleRef[] {
  const slice = model.slices.find((s) => s.entity.key === entity.key);
  if (!slice) return [];
  const out: StaleRef[] = [];
  const check = (kind: EntityKind, label: string, ref?: { id: EntityId }) => {
    if (!ref) return;
    // Match layout.ts buildLatestBySlug: highest loaded version for the
    // slug, not whichever envelope appears first in model.entities.
    let latest: Entity | undefined;
    for (const e of model.entities) {
      if (e.kind !== kind || e.id.namespace !== ref.id.namespace || e.id.slug !== ref.id.slug) continue;
      if (!latest || versionOf(e.id) > versionOf(latest.id)) latest = e;
    }
    if (latest && versionOf(latest.id) > versionOf(ref.id)) {
      out.push({ label, ref: ref.id, latest: latest.id.version });
    }
  };
  check('command', 'command', slice.command);
  check('readModel', 'read model', slice.readModel);
  check('ui', 'ui', slice.ui);
  check('persona', 'persona', slice.persona);
  check('processor', 'processor', slice.processor);
  for (const e of slice.events) check('event', 'event', e);
  for (const r of slice.readModels) check('readModel', 'source read model', r);
  return out;
}

// Problem-space knowledge graph (Decision #29): classification on the
// subdomain, realizes mapping on the context.
export function classificationOf(entity: Entity): string | undefined {
  if (entity.kind !== 'subdomain') return undefined;
  const c = String(entity.raw.classification ?? '');
  if (!c || c === 'CLASSIFICATION_UNSPECIFIED') return undefined;
  return c.replace('CLASSIFICATION_', '').toLowerCase();
}

// The agreement behind each seam (Decision #30), declared by the downstream
// context about its upstream.
export interface ContextRelationshipView {
  upstream: EntityId;
  intent: string;
  doc: string;
}

export function relationshipsOf(entity: Entity): ContextRelationshipView[] {
  if (entity.kind !== 'boundedContext') return [];
  const raw = Array.isArray(entity.raw.relationships) ? (entity.raw.relationships as Record<string, unknown>[]) : [];
  return raw
    .map((r) => ({
      upstream: asId((r.upstream as Record<string, unknown> | undefined)?.id),
      intent: String(r.intent ?? '')
        .replace('INTENT_', '')
        .replaceAll('_', ' ')
        .toLowerCase(),
      doc: String(r.doc ?? ''),
    }))
    .filter((r) => r.upstream.slug);
}

export function realizesOf(entity: Entity): EntityId[] {
  if (entity.kind !== 'boundedContext') return [];
  const raw = Array.isArray(entity.raw.realizes) ? (entity.raw.realizes as Record<string, unknown>[]) : [];
  return raw.map((r) => asId(r.id)).filter((i) => i.slug);
}

export function domainOf(entity: Entity): EntityId | undefined {
  if (entity.kind !== 'subdomain') return undefined;
  const d = (entity.raw.domain as Record<string, unknown> | undefined)?.id;
  const id = asId(d);
  return id.slug ? id : undefined;
}

export function realizedBy(model: Model, subdomain: Entity): Entity[] {
  return model.entities.filter(
    (e) =>
      e.kind === 'boundedContext' &&
      realizesOf(e).some((r) => r.namespace === subdomain.id.namespace && r.slug === subdomain.id.slug),
  );
}

export function subdomainsOf(model: Model, domain: Entity): Entity[] {
  return model.entities.filter((e) => {
    if (e.kind !== 'subdomain') return false;
    const d = domainOf(e);
    return d?.namespace === domain.id.namespace && d?.slug === domain.id.slug;
  });
}

// Declared pure navigation on a UI (proto UI.transitions): tab bars, back
// affordances, deep links; hops no command path explains.
export interface UiTransitionView {
  to: EntityId;
  doc: string;
}

// A command is intent; events are the facts. Mirrors the server-side rule
// COMMAND_NO_EMITTED_EVENTS: a Command must be wrapped by ≥1 CommandSlice
// that declares ≥1 emitted event, else it points nowhere. AutomationSlice
// emitting the command is not enough: the automation says who triggers it,
// the CommandSlice says what facts result.
export function commandsWithoutEmittedEvents(model: Model): Set<string> {
  const emitting = new Set<string>();
  for (const s of model.slices) {
    if (s.kind !== 'commandSlice') continue;
    if (!s.command || s.events.length === 0) continue;
    emitting.add(`${s.command.id.namespace}/${s.command.id.slug}`);
  }
  const broken = new Set<string>();
  for (const e of model.entities) {
    if (e.kind !== 'command') continue;
    if (!emitting.has(`${e.id.namespace}/${e.id.slug}`)) broken.add(e.key);
  }
  return broken;
}

// COMMAND_MULTIPLE_ISSUERS: a Command must be issued by at most one UI or
// processor across the whole model. CommandSlices declare the human issuer
// (ui), AutomationSlices declare the machine issuer (processor); if two
// different UIs or two different processors (or a UI plus a processor) point
// at the same command, the model is ambiguous about who can fire it. Repeated
// references from the same issuer count as one.
export function commandsWithMultipleIssuers(model: Model): Set<string> {
  const issuers = new Map<string, Set<string>>();
  const track = (cmdSlug: string, issuerKey: string) => {
    const set = issuers.get(cmdSlug) ?? new Set<string>();
    set.add(issuerKey);
    issuers.set(cmdSlug, set);
  };
  for (const s of model.slices) {
    if (!s.command) continue;
    const cmdSlug = `${s.command.id.namespace}/${s.command.id.slug}`;
    if (s.kind === 'commandSlice' && s.ui) {
      track(cmdSlug, `ui:${s.ui.id.namespace}/${s.ui.id.slug}`);
    } else if (s.kind === 'automationSlice' && s.processor) {
      track(cmdSlug, `processor:${s.processor.id.namespace}/${s.processor.id.slug}`);
    }
  }
  const broken = new Set<string>();
  for (const e of model.entities) {
    if (e.kind !== 'command') continue;
    const set = issuers.get(`${e.id.namespace}/${e.id.slug}`);
    if (set && set.size > 1) broken.add(e.key);
  }
  return broken;
}

// OrphanAnnotation (Decision #31): if the entity carries one, return its
// `doc`. Renderers should treat this as the explanation for why the
// entity is intentionally disconnected from the model's slice graph.
// An annotation with empty/missing doc returns ''. Absence returns
// undefined.
export function orphanOf(entity: Entity): string | undefined {
  const ann = entity.annotations.find((a) => a.type === 'OrphanAnnotation');
  if (!ann) return undefined;
  const doc = (ann.data as { doc?: unknown })?.doc;
  return typeof doc === 'string' ? doc : '';
}

export function uiTransitionsOf(entity: Entity): UiTransitionView[] {
  if (entity.kind !== 'ui') return [];
  const raw = Array.isArray(entity.raw.transitions) ? (entity.raw.transitions as Record<string, unknown>[]) : [];
  return raw
    .map((t) => ({ to: asId((t.to as Record<string, unknown> | undefined)?.id), doc: String(t.doc ?? '') }))
    .filter((t) => t.to.slug);
}

export interface ContinuationView {
  from: Entity;
  to: Entity;
  doc: string;
  // Any annotations on the transition's Next: the fork branch is a real
  // connection and carries design data like every other edge.
  metadata: unknown[];
  afterEvent: EntityId;
}

// One tracker's opinion about one model entity. Project management layers
// on top of the model (Decision #25): status never lives in design entities.
export interface TrackingView {
  tracker: Entity;
  status: string; // humanized, e.g. "in progress", "blocked"
  doc: string;
}

export interface Model {
  entities: Entity[];
  byKey: Map<string, Entity>;
  byKind: Map<EntityKind, Entity[]>;
  slices: SliceView[];
  storyboards: Entity[];
  continuations: ContinuationView[];
  swimlanes: Entity[];
  // Subject slugKey -> every tracker entry pointing at it.
  tracking: Map<string, TrackingView[]>;
  namespaces: string[];
}

const KIND_SET: ReadonlySet<string> = new Set([
  'event',
  'command',
  'readModel',
  'processor',
  'ui',
  'persona',
  'swimlane',
  'commandSlice',
  'readModelSlice',
  'automationSlice',
  'uiSlice',
  'storyboard',
  'eventModel',
  'component',
  'externalSystem',
  'tracker',
  'boundedContext',
  'domain',
  'subdomain',
  'schema',
  'project',
  'screen',
  'term',
  'ambiguity',
  'serviceLevelIndicator',
  'serviceLevelObjective',
  'alertPolicy',
  'alertNotificationTarget',
  'typeLibrary',
]);

const WIRE_KIND: Record<string, EntityKind> = {
  ENTITY_KIND_EVENT: 'event',
  ENTITY_KIND_COMMAND: 'command',
  ENTITY_KIND_READ_MODEL: 'readModel',
  ENTITY_KIND_PROCESSOR: 'processor',
  ENTITY_KIND_UI: 'ui',
  ENTITY_KIND_PERSONA: 'persona',
  ENTITY_KIND_SWIMLANE: 'swimlane',
  ENTITY_KIND_COMMAND_SLICE: 'commandSlice',
  ENTITY_KIND_READ_MODEL_SLICE: 'readModelSlice',
  ENTITY_KIND_AUTOMATION_SLICE: 'automationSlice',
  ENTITY_KIND_UI_SLICE: 'uiSlice',
  ENTITY_KIND_STORYBOARD: 'storyboard',
  ENTITY_KIND_EVENT_MODEL: 'eventModel',
  ENTITY_KIND_COMPONENT: 'component',
  ENTITY_KIND_EXTERNAL_SYSTEM: 'externalSystem',
  ENTITY_KIND_TRACKER: 'tracker',
  ENTITY_KIND_BOUNDED_CONTEXT: 'boundedContext',
  ENTITY_KIND_DOMAIN: 'domain',
  ENTITY_KIND_SUBDOMAIN: 'subdomain',
  ENTITY_KIND_SCHEMA: 'schema',
  ENTITY_KIND_PROJECT: 'project',
  ENTITY_KIND_SCREEN: 'screen',
  ENTITY_KIND_TERM: 'term',
  ENTITY_KIND_AMBIGUITY: 'ambiguity',
  ENTITY_KIND_SERVICE_LEVEL_INDICATOR: 'serviceLevelIndicator',
  ENTITY_KIND_SERVICE_LEVEL_OBJECTIVE: 'serviceLevelObjective',
  ENTITY_KIND_ALERT_POLICY: 'alertPolicy',
  ENTITY_KIND_ALERT_NOTIFICATION_TARGET: 'alertNotificationTarget',
  ENTITY_KIND_TYPE_LIBRARY: 'typeLibrary',
};

export function wireKind(wire: unknown): EntityKind | undefined {
  return WIRE_KIND[String(wire ?? '')];
}

export function humanStatus(wire: unknown): string {
  return String(wire ?? '')
    .replace('TRACK_STATUS_', '')
    .toLowerCase()
    .replace(/_/g, ' ');
}

export function entityKey(kind: EntityKind, id: EntityId): string {
  return `${kind}:${id.namespace}/${id.slug}@${id.version}`;
}

/** Key ignoring version, for slice refs that point at "the entity". */
export function slugKey(kind: EntityKind, id: { namespace: string; slug: string }): string {
  return `${kind}:${id.namespace}/${id.slug}`;
}

export interface ScreenSlotView {
  name: string;
  doc: string;
  required: boolean;
  contributors: Entity[];
}

export function screenSlotsOf(model: Model, screen: Entity): ScreenSlotView[] {
  if (screen.kind !== 'screen') return [];
  const slots = Array.isArray(screen.raw.slots) ? (screen.raw.slots as Record<string, unknown>[]) : [];
  return slots.map((slot) => {
    const name = String(slot.name ?? '');
    return {
      name,
      doc: String(slot.doc ?? ''),
      required: Boolean(slot.required),
      contributors: model.entities.filter(
        (e) =>
          e.kind === 'ui' &&
          e.screenSlot?.screen.namespace === screen.id.namespace &&
          e.screenSlot.screen.slug === screen.id.slug &&
          e.screenSlot.slot === name,
      ),
    };
  });
}

export function screenOfUi(model: Model, ui: Entity): Entity | undefined {
  if (ui.kind !== 'ui' || !ui.screenSlot) return undefined;
  const slot = ui.screenSlot;
  return model.entities.find(
    (e) => e.kind === 'screen' && e.id.namespace === slot.screen.namespace && e.id.slug === slot.screen.slug,
  );
}

export function termEmbodiedBy(entity: Entity): { kind: EntityKind; id: EntityId }[] {
  if (entity.kind !== 'term') return [];
  const refs = Array.isArray(entity.raw.embodiedBy) ? (entity.raw.embodiedBy as Record<string, unknown>[]) : [];
  return refs
    .map((ref) => {
      const kind = wireKind(ref.kind);
      const id = asId(ref.id);
      return kind && id.slug ? { kind, id } : undefined;
    })
    .filter((x): x is { kind: EntityKind; id: EntityId } => Boolean(x));
}

export function ambiguityTermsOf(entity: Entity): EntityId[] {
  if (entity.kind !== 'ambiguity') return [];
  const refs = Array.isArray(entity.raw.terms) ? (entity.raw.terms as Record<string, unknown>[]) : [];
  return refs.map((ref) => asId(ref.id)).filter((id) => id.slug);
}

export function ambiguityRulingOf(entity: Entity): string | undefined {
  if (entity.kind !== 'ambiguity') return undefined;
  const kind = String(entity.raw.kind ?? '')
    .replace('KIND_', '')
    .toLowerCase();
  const ruling = String(entity.raw.ruling ?? '')
    .replace('RULING_', '')
    .toLowerCase();
  return [kind, ruling].filter((part) => part && part !== 'unspecified').join(' ');
}

type WireEntity = { kind?: string } & Record<string, unknown>;

export function asId(raw: unknown): EntityId {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    return { namespace: '', slug: '', version: '0' };
  }
  const id = raw as Partial<EntityId>;
  return {
    namespace: id.namespace ?? '',
    slug: id.slug ?? '',
    version: String(id.version ?? '0'),
  };
}

function refEdge(edge: unknown, refField: string): EdgeRef | undefined {
  if (!edge || typeof edge !== 'object') return undefined;
  const body = edge as Record<string, unknown>;
  const ref = body[refField];
  if (!ref || typeof ref !== 'object') return undefined;
  const id = (ref as Record<string, unknown>).id;
  if (!id) return undefined;
  return {
    id: asId(id),
    doc: String(body.doc ?? ''),
    metadata: Array.isArray(body.metadata) ? body.metadata : [],
  };
}

/**
 * Reads the Any-wrapped Schema's FieldSpec list from `bodyObj.schema`.
 * Named user message types (non-Schema Anys) have no FieldSpec list.
 */
function fieldsOf(bodyObj: Record<string, unknown>): FieldSpec[] {
  const schema = bodyObj.schema;
  if (schema && typeof schema === 'object' && !Array.isArray(schema)) {
    const s = schema as Record<string, unknown>;
    const typeUrl = s['@type'];
    const isSchema =
      typeof typeUrl !== 'string' ||
      typeUrl.endsWith('trogonatlas.eventmodel.v1alpha1.Schema') ||
      typeUrl.endsWith('/Schema');
    if (isSchema && Array.isArray(s.fields)) return s.fields as FieldSpec[];
  }
  return [];
}

function normalizeEntity(wire: WireEntity): Entity | undefined {
  const kind = (Object.keys(wire) as EntityKind[]).find((k) => KIND_SET.has(k));
  if (!kind) return undefined;
  const body = wire[kind];
  if (!body || typeof body !== 'object' || Array.isArray(body)) return undefined;
  const bodyObj = body as Record<string, unknown>;
  const system = wire.system as { uid?: string; createdAt?: string } | undefined;
  const id = asId(bodyObj.id);
  const swimlaneRef = bodyObj.swimlane as Record<string, unknown> | undefined;
  const ext = bodyObj.externalSource as Record<string, unknown> | undefined;
  const slot = bodyObj.slot as Record<string, unknown> | undefined;
  const slotScreen = slot?.screen as Record<string, unknown> | undefined;
  const screenSlot =
    slot && slotScreen?.id
      ? {
          screen: asId(slotScreen.id),
          slot: String(slot.slot ?? ''),
        }
      : undefined;
  return {
    kind,
    id,
    key: entityKey(kind, id),
    title: String(bodyObj.title ?? id.slug),
    doc: String(bodyObj.doc ?? ''),
    role: bodyObj.role ? String(bodyObj.role) : undefined,
    order: bodyObj.order !== undefined ? Number(bodyObj.order) : undefined,
    color: bodyObj.color ? String(bodyObj.color) : undefined,
    swimlane: swimlaneRef?.id ? asId(swimlaneRef.id) : undefined,
    externalSource: ext
      ? {
          system: String((ext.system as { id?: { slug?: string } } | undefined)?.id?.slug ?? ''),
          descriptor: String(ext.descriptor ?? ''),
        }
      : undefined,
    tech: Array.isArray(bodyObj.tech) ? (bodyObj.tech as unknown[]).map(String) : undefined,
    calls: Array.isArray(bodyObj.calls)
      ? (bodyObj.calls as Record<string, unknown>[]).map((c) => asId(c.id)).filter((i) => i.slug)
      : undefined,
    streamId: bodyObj.streamId ? String(bodyObj.streamId) : undefined,
    screenSlot,
    uid: system?.uid ? String(system.uid) : undefined,
    createdAt: system?.createdAt ? String(system.createdAt) : undefined,
    annotations: decodeAnnotations(bodyObj.metadata),
    fields: fieldsOf(bodyObj),
    raw: bodyObj,
  };
}

/**
 * Scenario example payloads arrive as decoded Anys from the bridge:
 *   - Struct: `{ "@type": "...Struct", value: { ... } }` (pre-codegen)
 *   - Generated message: `{ "@type": "...Place", total: 42, ... }`
 *   - Already-unwrapped plain object (tests / tooling): `{ total: 42 }`
 */
function examplePayload(p: unknown): Record<string, unknown> | undefined {
  if (!p || typeof p !== 'object' || Array.isArray(p)) return undefined;
  const o = p as Record<string, unknown>;
  const t = o['@type'];
  if (typeof t === 'string' && t.endsWith('google.protobuf.Struct')) {
    return o.value && typeof o.value === 'object' && !Array.isArray(o.value)
      ? (o.value as Record<string, unknown>)
      : undefined;
  }
  if (typeof t === 'string') {
    const { '@type': _ignored, ...data } = o;
    return Object.keys(data).length > 0 ? data : undefined;
  }
  return o;
}

function example(kind: ScenarioExample['kind'], raw: unknown, refField: string): ScenarioExample | undefined {
  if (!raw || typeof raw !== 'object') return undefined;
  const body = raw as Record<string, unknown>;
  const ref = body[refField] as Record<string, unknown> | undefined;
  if (!ref?.id) return undefined;
  return { kind, id: asId(ref.id), data: examplePayload(body.payload) };
}

function exampleList(kind: ScenarioExample['kind'], raw: unknown, refField: string): ScenarioExample[] {
  return (Array.isArray(raw) ? raw : [])
    .map((e) => example(kind, e, refField))
    .filter((x): x is ScenarioExample => Boolean(x));
}

function scenarioViews(entity: Entity): ScenarioView[] {
  const list = Array.isArray(entity.raw.scenarios) ? entity.raw.scenarios : [];
  return list.map((raw) => {
    const s = raw as Record<string, unknown>;
    const v: ScenarioView = {
      id: String(s.id ?? ''),
      title: String(s.title ?? ''),
      doc: String(s.doc ?? ''),
      given: [],
      when: [],
      thenEmits: [],
      annotations: decodeAnnotations(s.metadata),
    };
    if (entity.kind === 'commandSlice') {
      v.given = exampleList('event', s.given, 'event');
      const w = example('command', s.when, 'command');
      if (w) v.when = [w];
      const emit = s.emit as Record<string, unknown> | undefined;
      if (emit) v.thenEmits = exampleList('event', emit.events, 'event');
      const reject = s.reject as Record<string, unknown> | undefined;
      if (reject) v.reject = { reasonCode: String(reject.reasonCode ?? ''), doc: String(reject.doc ?? '') };
    } else if (entity.kind === 'readModelSlice') {
      v.when = exampleList('event', s.when, 'event');
      const t = example('readModel', s.then, 'readModel');
      if (t) v.thenState = t;
    } else if (entity.kind === 'automationSlice') {
      v.given = exampleList('readModel', s.given, 'readModel');
      v.whenDoc = s.whenDoc ? String(s.whenDoc) : undefined;
      const t = example('command', s.then, 'command');
      if (t) v.thenCommand = t;
    }
    return v;
  });
}

function sliceView(entity: Entity): SliceView | undefined {
  const body = entity.raw;
  if (entity.kind === 'commandSlice') {
    return {
      kind: 'commandSlice',
      entity,
      persona: refEdge(body.persona, 'persona'),
      ui: refEdge(body.ui, 'ui'),
      command: refEdge(body.command, 'command'),
      events: (Array.isArray(body.emittedEvents) ? body.emittedEvents : [])
        .map((e) => refEdge(e, 'event'))
        .filter((x): x is EdgeRef => Boolean(x)),
      readModels: [],
      scenarios: scenarioViews(entity),
    };
  }
  if (entity.kind === 'readModelSlice') {
    return {
      kind: 'readModelSlice',
      entity,
      readModel: refEdge(body.readModel, 'readModel'),
      events: (Array.isArray(body.sourceEvents) ? body.sourceEvents : [])
        .map((e) => refEdge(e, 'event'))
        .filter((x): x is EdgeRef => Boolean(x)),
      readModels: [],
      scenarios: scenarioViews(entity),
    };
  }
  if (entity.kind === 'automationSlice') {
    return {
      kind: 'automationSlice',
      entity,
      processor: refEdge(body.processor, 'processor'),
      command: refEdge(body.emittedCommand, 'command'),
      events: [],
      readModels: (Array.isArray(body.sourceReadModels) ? body.sourceReadModels : [])
        .map((e) => refEdge(e, 'readModel'))
        .filter((x): x is EdgeRef => Boolean(x)),
      scenarios: scenarioViews(entity),
    };
  }
  if (entity.kind === 'uiSlice') {
    return {
      kind: 'uiSlice',
      entity,
      persona: refEdge(body.persona, 'persona'),
      ui: refEdge(body.ui, 'ui'),
      events: [],
      readModels: (Array.isArray(body.sourceReadModels) ? body.sourceReadModels : [])
        .map((e) => refEdge(e, 'readModel'))
        .filter((x): x is EdgeRef => Boolean(x)),
      scenarios: [],
    };
  }
  return undefined;
}

export function buildModel(wireEntities: WireEntity[]): Model {
  return assembleModel(wireEntities.map(normalizeEntity).filter((x): x is Entity => Boolean(x)));
}

/**
 * One Model per namespace. A bounded context is its own thing (its own
 * personas, lanes, and timeline), never merged into one gigantic board;
 * groups touch only through context seams.
 */
export function partitionByNamespace(model: Model): Model[] {
  if (model.namespaces.length <= 1) return [model];
  return model.namespaces.map((ns) => assembleModel(model.entities.filter((e) => e.id.namespace === ns)));
}

// Storyboards (sub-workflows) follow the event model's curated members
// order: that ordered listing is how the author sequences the chunks.
function buildStoryboardOrder(entities: Entity[]): Map<string, number> {
  const order = new Map<string, number>();
  let idx = 0;
  for (const em of entities.filter((e) => e.kind === 'eventModel')) {
    const members = Array.isArray(em.raw.members) ? em.raw.members : [];
    for (const m of members) {
      const member = m as Record<string, unknown>;
      if (member.kind !== 'ENTITY_KIND_STORYBOARD') continue;
      const id = asId(member.id);
      const k = `${id.namespace}/${id.slug}`;
      if (!order.has(k)) order.set(k, idx++);
    }
  }
  return order;
}

// Forks are DERIVED, never declared on storyboards: the swimlane's
// transition table says which event has alternative successors; the
// storyboard whose outcome emits `after` forks into the storyboards whose
// slices emit each `next`. Build the two reverse indices the continuation
// derivation walks: outcome-event → storyboard, and emitted-event →
// storyboard.
function buildStoryboardEventIndices(
  storyboards: Entity[],
  slices: SliceView[],
): { byOutcomeEvent: Map<string, Entity>; byEmittedEvent: Map<string, Entity> } {
  const byOutcomeEvent = new Map<string, Entity>();
  const byEmittedEvent = new Map<string, Entity>();
  for (const sb of storyboards) {
    const outcome = sb.raw.outcome as Record<string, unknown> | undefined;
    for (const e of Array.isArray(outcome?.events) ? outcome.events : []) {
      const ref = (e as Record<string, unknown>).event as Record<string, unknown> | undefined;
      if (!ref?.id) continue;
      const id = asId(ref.id);
      const k = `${id.namespace}/${id.slug}`;
      if (!byOutcomeEvent.has(k)) byOutcomeEvent.set(k, sb);
    }
  }
  const sliceToSb = new Map<string, Entity>();
  for (const sb of storyboards) {
    for (const ref of Array.isArray(sb.raw.slices) ? sb.raw.slices : []) {
      const id = asId((ref as Record<string, unknown>)?.id);
      sliceToSb.set(`${id.namespace}/${id.slug}`, sb);
    }
  }
  for (const s of slices) {
    if (s.kind !== 'commandSlice') continue;
    const sb = sliceToSb.get(`${s.entity.id.namespace}/${s.entity.id.slug}`);
    if (!sb) continue;
    for (const ev of s.events) {
      const k = `${ev.id.namespace}/${ev.id.slug}`;
      if (!byEmittedEvent.has(k)) byEmittedEvent.set(k, sb);
    }
  }
  return { byOutcomeEvent, byEmittedEvent };
}

function buildContinuations(
  entities: Entity[],
  byOutcomeEvent: Map<string, Entity>,
  byEmittedEvent: Map<string, Entity>,
): ContinuationView[] {
  const continuations: ContinuationView[] = [];
  for (const lane of entities.filter((e) => e.kind === 'swimlane')) {
    for (const t of Array.isArray(lane.raw.transitions) ? lane.raw.transitions : []) {
      const trans = t as Record<string, unknown>;
      const afterRef = (trans.after as Record<string, unknown> | undefined)?.id;
      if (!afterRef) continue;
      const afterId = asId(afterRef);
      const afterKey = `${afterId.namespace}/${afterId.slug}`;
      const from = byOutcomeEvent.get(afterKey) ?? byEmittedEvent.get(afterKey);
      if (!from) continue;
      for (const n of Array.isArray(trans.next) ? trans.next : []) {
        const next = n as Record<string, unknown>;
        const nextRef = (next.event as Record<string, unknown> | undefined)?.id;
        if (!nextRef) continue;
        const nextId = asId(nextRef);
        const to = byEmittedEvent.get(`${nextId.namespace}/${nextId.slug}`);
        if (!to || to.key === from.key) continue;
        continuations.push({
          from,
          to,
          doc: String(next.doc ?? ''),
          metadata: Array.isArray(next.metadata) ? next.metadata : [],
          afterEvent: afterId,
        });
      }
    }
  }
  return continuations;
}

function buildTrackingMap(entities: Entity[]): Map<string, TrackingView[]> {
  const tracking = new Map<string, TrackingView[]>();
  for (const tracker of entities.filter((e) => e.kind === 'tracker')) {
    for (const raw of Array.isArray(tracker.raw.items) ? tracker.raw.items : []) {
      const item = raw as Record<string, unknown>;
      const subject = item.subject as Record<string, unknown> | undefined;
      const kind = WIRE_KIND[String(subject?.kind ?? '')];
      if (!kind || !subject?.id) continue;
      const k = slugKey(kind, asId(subject.id));
      const list = tracking.get(k) ?? [];
      list.push({ tracker, status: humanStatus(item.status), doc: String(item.doc ?? '') });
      tracking.set(k, list);
    }
  }
  return tracking;
}

export function assembleModel(entities: Entity[]): Model {
  const byKey = new Map(entities.map((e) => [e.key, e]));
  const byKind = new Map<EntityKind, Entity[]>();
  for (const e of entities) {
    const list = byKind.get(e.kind) ?? [];
    list.push(e);
    byKind.set(e.kind, list);
  }
  const slices = entities.map(sliceView).filter((x): x is SliceView => Boolean(x));
  const sbOrder = buildStoryboardOrder(entities);
  const storyboards = entities
    .filter((e) => e.kind === 'storyboard')
    .sort(
      (a, b) =>
        (sbOrder.get(`${a.id.namespace}/${a.id.slug}`) ?? Number.MAX_SAFE_INTEGER) -
        (sbOrder.get(`${b.id.namespace}/${b.id.slug}`) ?? Number.MAX_SAFE_INTEGER),
    );
  const { byOutcomeEvent, byEmittedEvent } = buildStoryboardEventIndices(storyboards, slices);
  const continuations = buildContinuations(entities, byOutcomeEvent, byEmittedEvent);
  const tracking = buildTrackingMap(entities);
  const swimlanes = entities.filter((e) => e.kind === 'swimlane').sort((a, b) => (a.order ?? 0) - (b.order ?? 0));
  const namespaces = [...new Set(entities.map((e) => e.id.namespace))].sort();
  return { entities, byKey, byKind, slices, storyboards, continuations, swimlanes, tracking, namespaces };
}

/** Tracker entries pointing at this entity (status overlays). */
export function trackingOf(model: Model, entity: Entity): TrackingView[] {
  return model.tracking.get(slugKey(entity.kind, entity.id)) ?? [];
}

/** The read model a storyboard's entry observes: where a fork is drawn from. */
export function storyboardEntryReadModel(sb: Entity): EntityId | undefined {
  const entry = sb.raw.entry as Record<string, unknown> | undefined;
  const edge = entry?.readModel as Record<string, unknown> | undefined;
  const ref = edge?.readModel as Record<string, unknown> | undefined;
  return ref?.id ? asId(ref.id) : undefined;
}

/**
 * The persona / ui / processor that observes the storyboard at its entry.
 * Anchors layout: the entry observer's surfaces are placed at the
 * storyboard's first column instead of drifting to the "unwired" gutter
 * when no slice happens to reference them.
 */
export interface StoryboardEntryObserver {
  persona?: EntityId;
  ui?: EntityId;
  processor?: EntityId;
}
export function storyboardEntryObserver(sb: Entity): StoryboardEntryObserver {
  const entry = sb.raw.entry as Record<string, unknown> | undefined;
  const human = entry?.human as Record<string, unknown> | undefined;
  const auto = entry?.automation as Record<string, unknown> | undefined;
  const personaRef = (human?.persona as Record<string, unknown> | undefined)?.persona as
    | Record<string, unknown>
    | undefined;
  const uiRef = (human?.ui as Record<string, unknown> | undefined)?.ui as Record<string, unknown> | undefined;
  const procRef = (auto?.processor as Record<string, unknown> | undefined)?.processor as
    | Record<string, unknown>
    | undefined;
  return {
    persona: personaRef?.id ? asId(personaRef.id) : undefined,
    ui: uiRef?.id ? asId(uiRef.id) : undefined,
    processor: procRef?.id ? asId(procRef.id) : undefined,
  };
}

/** Ordered slice keys: storyboard order first, leftovers by slug. */
export function orderedSlices(model: Model): SliceView[] {
  const bySlug = new Map(model.slices.map((s) => [`${s.entity.id.namespace}/${s.entity.id.slug}`, s]));
  const ordered: SliceView[] = [];
  const seen = new Set<string>();
  for (const sb of model.storyboards) {
    const refs = Array.isArray(sb.raw.slices) ? sb.raw.slices : [];
    for (const ref of refs) {
      const id = asId((ref as Record<string, unknown>)?.id);
      const k = `${id.namespace}/${id.slug}`;
      const slice = bySlug.get(k);
      if (slice && !seen.has(k)) {
        seen.add(k);
        ordered.push(slice);
      }
    }
  }
  const rest = model.slices
    .filter((s) => !seen.has(`${s.entity.id.namespace}/${s.entity.id.slug}`))
    .sort((a, b) => a.entity.id.slug.localeCompare(b.entity.id.slug));
  return [...ordered, ...rest];
}

// One step of temporal navigation: a neighbor of the selected entity on the
// timeline, with the EM relation that connects them.
export interface NeighborView {
  entity: Entity;
  relation: string;
}

/**
 * The selected entity's temporal neighbors, derived from the slices, the
 * swimlane lifecycles, and the storyboard fork graph. Backward = what leads
 * into this moment; forward = what this moment leads into. Drives drawer
 * navigation: click a neighbor to select, highlight, and pan to it.
 */
export function neighborsOf(model: Model, entity: Entity): { backward: NeighborView[]; forward: NeighborView[] } {
  const bySlug = new Map(model.entities.map((e) => [slugKey(e.kind, e.id), e]));
  const resolve = (kind: EntityKind, ref: { id: { namespace: string; slug: string } } | undefined) =>
    ref ? bySlug.get(slugKey(kind, ref.id)) : undefined;
  const me = slugKey(entity.kind, entity.id);
  const is = (e: Entity | undefined) => Boolean(e && slugKey(e.kind, e.id) === me);

  const backward: NeighborView[] = [];
  const forward: NeighborView[] = [];
  const add = (list: NeighborView[], e: Entity | undefined, relation: string) => {
    if (!e || slugKey(e.kind, e.id) === me) return;
    if (list.some((n) => n.entity.key === e.key)) return;
    list.push({ entity: e, relation });
  };

  for (const s of model.slices) {
    if (s.kind === 'commandSlice') {
      const ui = resolve('ui', s.ui);
      const cmd = resolve('command', s.command);
      const events = s.events.map((e) => resolve('event', e)).filter((e): e is Entity => Boolean(e));
      if (is(ui)) add(forward, cmd, 'issues');
      if (is(cmd)) {
        add(backward, ui, 'triggered from');
        for (const ev of events) add(forward, ev, 'emits');
      }
      for (const ev of events) if (is(ev)) add(backward, cmd, 'emitted by');
    } else if (s.kind === 'readModelSlice') {
      const rm = resolve('readModel', s.readModel);
      const events = s.events.map((e) => resolve('event', e)).filter((e): e is Entity => Boolean(e));
      for (const ev of events) {
        const cross = Boolean(rm && ev && rm.id.namespace !== ev.id.namespace);
        if (is(ev)) add(forward, rm, cross ? `feeds context ${rm?.id.namespace}` : 'projects into');
        if (is(rm)) add(backward, ev, cross ? `translated from context ${ev.id.namespace}` : 'projected from');
      }
    } else if (s.kind === 'uiSlice') {
      const ui = resolve('ui', s.ui);
      const rms = s.readModels.map((r) => resolve('readModel', r)).filter((e): e is Entity => Boolean(e));
      for (const rm of rms) {
        if (is(rm)) add(forward, ui, 'renders into');
        if (is(ui)) add(backward, rm, 'shows');
      }
    } else {
      const proc = resolve('processor', s.processor);
      const cmd = resolve('command', s.command);
      const rms = s.readModels.map((r) => resolve('readModel', r)).filter((e): e is Entity => Boolean(e));
      for (const rm of rms) {
        if (is(rm)) add(forward, proc, 'observed by');
        if (is(proc)) add(backward, rm, 'observes');
      }
      if (is(proc)) add(forward, cmd, 'issues');
      if (is(cmd)) add(backward, proc, 'issued by');
    }
  }

  // The stream's lifecycle: an event's legal successors and predecessors.
  for (const lane of model.byKind.get('swimlane') ?? []) {
    for (const t of Array.isArray(lane.raw.transitions) ? (lane.raw.transitions as Record<string, unknown>[]) : []) {
      const after = resolve('event', t.after as { id: EntityId } | undefined);
      for (const n of Array.isArray(t.next) ? (t.next as Record<string, unknown>[]) : []) {
        const next = resolve('event', n.event as { id: EntityId } | undefined);
        if (is(after)) add(forward, next, 'lifecycle: then');
        if (is(next)) add(backward, after, 'lifecycle: after');
      }
    }
  }

  // The external round trip: a processor calls a system; the system's
  // webhook lands in the ExternalSource views pointing back at it.
  if (entity.kind === 'processor') {
    for (const c of entity.calls ?? []) add(forward, bySlug.get(slugKey('externalSystem', c)), 'calls (external)');
  }
  if (entity.kind === 'externalSystem') {
    for (const p of model.byKind.get('processor') ?? []) {
      if ((p.calls ?? []).some((c) => slugKey('externalSystem', c) === me)) add(backward, p, 'called by');
    }
    for (const rm of model.byKind.get('readModel') ?? []) {
      const src = (rm.raw.externalSource as { system?: { id?: unknown } } | undefined)?.system?.id;
      if (src && slugKey('externalSystem', asId(src)) === me) add(forward, rm, 'returns via webhook into');
    }
  }

  // Storyboards chain through the fork graph; slices walk the chunk order.
  for (const c of model.continuations) {
    if (c.from.key === entity.key) add(forward, c.to, c.doc ? `forks into, when ${c.doc}` : 'forks into');
    if (c.to.key === entity.key) add(backward, c.from, c.doc ? `continues from, when ${c.doc}` : 'continues from');
  }
  if (
    entity.kind === 'commandSlice' ||
    entity.kind === 'readModelSlice' ||
    entity.kind === 'automationSlice' ||
    entity.kind === 'uiSlice'
  ) {
    const ordered = orderedSlices(model);
    const idx = ordered.findIndex((s) => s.entity.key === entity.key);
    if (idx > 0) add(backward, ordered[idx - 1].entity, 'previous moment');
    if (idx >= 0 && idx < ordered.length - 1) add(forward, ordered[idx + 1].entity, 'next moment');
  }

  return { backward, forward };
}

// A context seam: the ONE legal touchpoint between bounded contexts, an
// upstream event flowing into a downstream subscription read model. Derived
// purely from cross-namespace source_events refs; the upstream entity may
// not be loaded (single-namespace board), so the upstream side is an ID.
export interface ContextSeam {
  upstreamEvent: EntityId;
  view: Entity;
}

export function contextSeams(model: Model): ContextSeam[] {
  const seams: ContextSeam[] = [];
  for (const rm of model.entities.filter((e) => e.kind === 'readModel')) {
    for (const ref of Array.isArray(rm.raw.sourceEvents) ? rm.raw.sourceEvents : []) {
      const id = asId((ref as Record<string, unknown>)?.id);
      if (id.slug && id.namespace && id.namespace !== rm.id.namespace) {
        seams.push({ upstreamEvent: id, view: rm });
      }
    }
  }
  return seams;
}

/** Upstream context namespaces feeding this subscription view. */
export function upstreamContextsOf(entity: Entity): string[] {
  if (entity.kind !== 'readModel') return [];
  const out = new Set<string>();
  for (const ref of Array.isArray(entity.raw.sourceEvents) ? entity.raw.sourceEvents : []) {
    const id = asId((ref as Record<string, unknown>)?.id);
    if (id.namespace && id.namespace !== entity.id.namespace) out.add(id.namespace);
  }
  return [...out];
}

/** Cross-context source event refs on a subscription view: full
 * {namespace, slug, version} per ref, so consumers can build "open in
 * upstream context" links instead of just naming the namespace. */
export function upstreamSourceEventsOf(entity: Entity): EntityId[] {
  if (entity.kind !== 'readModel') return [];
  const out: EntityId[] = [];
  for (const ref of Array.isArray(entity.raw.sourceEvents) ? entity.raw.sourceEvents : []) {
    const id = asId((ref as Record<string, unknown>)?.id);
    if (id.namespace && id.slug && id.namespace !== entity.id.namespace) out.push(id);
  }
  return out;
}

// Instance identity, scoped by type: an instance is derived (no birth
// event, no storage), so its id must be RECOMPUTABLE from the facts that
// define it; uuidv5 with the TYPE's incarnation uid as the namespace and
// the defining slice's uid as the name: "this moment, within this type's
// identity space". The nondeterministic entropy is inherited from the two
// server-minted v7 uids; determinism on top is what makes the id mean the
// same moment on every render, for every client. Recreate either
// incarnation and the instance id changes with it. Synthetic occurrences
// (no defining slice) honestly have none.
import { v5 as uuidv5 } from 'uuid';

export function instanceUidOf(type: Entity | undefined, moment: Entity | undefined): string | undefined {
  if (!type?.uid || !moment?.uid) return undefined;
  return uuidv5(moment.uid, type.uid);
}

/**
 * Temporal neighbors of one MOMENT (entity at its defining slice), not the
 * type: a queue instance projected by a clearing slice was not observed by
 * the automation that fired before it existed; only the latest state
 * before the automation is. Falls back to type-level when the moment is
 * unknown.
 */
export function neighborsOfMoment(
  model: Model,
  entity: Entity,
  momentKey: string,
): { backward: NeighborView[]; forward: NeighborView[] } {
  const s = model.slices.find((x) => x.entity.key === momentKey);
  if (!s) return neighborsOf(model, entity);
  const bySlug = new Map(model.entities.map((e) => [slugKey(e.kind, e.id), e]));
  const resolve = (kind: EntityKind, ref: { id: { namespace: string; slug: string } } | undefined) =>
    ref ? bySlug.get(slugKey(kind, ref.id)) : undefined;
  const me = slugKey(entity.kind, entity.id);
  const is = (e: Entity | undefined) => Boolean(e && slugKey(e.kind, e.id) === me);
  const backward: NeighborView[] = [];
  const forward: NeighborView[] = [];
  const add = (list: NeighborView[], e: Entity | undefined, relation: string) => {
    if (!e || slugKey(e.kind, e.id) === me) return;
    if (list.some((n) => n.entity.key === e.key)) return;
    list.push({ entity: e, relation });
  };

  // Relations belonging to THIS slice only.
  if (s.kind === 'commandSlice') {
    const ui = resolve('ui', s.ui);
    const cmd = resolve('command', s.command);
    const events = s.events.map((e) => resolve('event', e)).filter((e): e is Entity => Boolean(e));
    if (is(ui)) add(forward, cmd, 'issues');
    if (is(cmd)) {
      add(backward, ui, 'triggered from');
      for (const ev of events) add(forward, ev, 'emits');
    }
    for (const ev of events) if (is(ev)) add(backward, cmd, 'emitted by');
  } else if (s.kind === 'readModelSlice') {
    const rm = resolve('readModel', s.readModel);
    for (const e of s.events) {
      const ev = resolve('event', e);
      if (is(rm)) add(backward, ev, 'projected from');
      if (is(ev)) add(forward, rm, 'projects into');
    }
  } else if (s.kind === 'uiSlice') {
    const ui = resolve('ui', s.ui);
    if (is(ui)) {
      for (const r of s.readModels) add(backward, resolve('readModel', r), 'shows');
    }
  } else {
    const proc = resolve('processor', s.processor);
    const cmd = resolve('command', s.command);
    const rms = s.readModels.map((r) => resolve('readModel', r)).filter((e): e is Entity => Boolean(e));
    if (is(proc)) {
      for (const rm of rms) add(backward, rm, 'observes');
      add(forward, cmd, 'issues');
    }
  }

  // A consumer reads the LATEST state of its view before it runs: only that
  // instance gets the consumer edge; later instances came after.
  if (entity.kind === 'readModel') {
    const ordered = orderedSlices(model);
    const myIdx = ordered.findIndex((x) => x.entity.key === momentKey);
    const sameRm = (ref: EdgeRef | undefined) => ref && slugKey('readModel', ref.id) === me;
    for (const [idx, consumer] of ordered.entries()) {
      if (consumer.kind !== 'automationSlice' && consumer.kind !== 'uiSlice') continue;
      if (!consumer.readModels.some((r) => sameRm(r))) continue;
      let latest = -1;
      for (let i = 0; i < idx; i++) {
        if (ordered[i].kind === 'readModelSlice' && sameRm(ordered[i].readModel)) latest = i;
      }
      if (latest !== myIdx) continue;
      if (consumer.kind === 'automationSlice') add(forward, resolve('processor', consumer.processor), 'observed by');
      else add(forward, resolve('ui', consumer.ui), 'renders into');
    }
  }

  return { backward, forward };
}

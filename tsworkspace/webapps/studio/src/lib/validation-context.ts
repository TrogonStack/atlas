import type { Issue } from '@/lib/issues';
import { asId, type Entity, type EntityId, type EntityKind, entityKey, type Model, wireKind } from '@/lib/model';

type ContextRef = { kind: EntityKind | 'slice'; id: EntityId };

const sliceKinds = ['commandSlice', 'readModelSlice', 'automationSlice', 'uiSlice'] as const;
const referenceFields: Record<string, ContextRef['kind']> = {
  event: 'event',
  command: 'command',
  readModel: 'readModel',
  persona: 'persona',
  ui: 'ui',
  processor: 'processor',
  swimlane: 'swimlane',
  screen: 'screen',
  storyboard: 'storyboard',
  sourceEvents: 'event',
  emittedEvents: 'event',
  sourceReadModels: 'readModel',
  emittedCommand: 'command',
  slices: 'slice',
  slice: 'slice',
  calls: 'externalSystem',
  upstream: 'boundedContext',
  realizes: 'subdomain',
  domain: 'domain',
};

function identity(ref: ContextRef): string {
  return `${ref.kind}:${ref.id.namespace}/${ref.id.slug}@${ref.id.version}`;
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : undefined;
}

function references(entity: Entity): ContextRef[] {
  const found = new Map<string, ContextRef>();
  const add = (kind: ContextRef['kind'], value: unknown) => {
    const id = asId(value);
    if (id.namespace && id.slug) found.set(identity({ kind, id }), { kind, id });
  };
  const visit = (value: unknown, impliedKind?: ContextRef['kind']) => {
    if (Array.isArray(value)) {
      for (const item of value) visit(item, impliedKind);
      return;
    }
    const body = record(value);
    if (!body) return;
    const kind = wireKind(body.kind) ?? impliedKind;
    if (kind && body.id) add(kind, body.id);
    for (const [field, child] of Object.entries(body)) {
      if (['id', 'data', 'metadata', 'fields', 'schema', 'supersedes'].includes(field)) continue;
      visit(child, referenceFields[field]);
    }
  };
  visit(entity.raw);
  if (entity.raw.supersedes) add(entity.kind, entity.raw.supersedes);
  if (entity.kind === 'readModel') {
    const source = record(entity.raw.externalSource);
    add('externalSystem', record(source?.system)?.id);
  }
  if (entity.kind === 'swimlane' && Array.isArray(entity.raw.transitions)) {
    for (const transition of entity.raw.transitions) add('event', record(record(transition)?.after)?.id);
  }
  if (entity.kind === 'ui' && Array.isArray(entity.raw.transitions)) {
    for (const transition of entity.raw.transitions) add('ui', record(record(transition)?.to)?.id);
  }
  return [...found.values()];
}

function resolve(model: Model, ref: ContextRef): Entity[] {
  const kinds = ref.kind === 'slice' ? sliceKinds : [ref.kind];
  return kinds.flatMap((kind) => {
    const entity = model.byKey.get(entityKey(kind, ref.id));
    return entity ? [entity] : [];
  });
}

function storyboardSlices(entity: Entity): ContextRef[] {
  if (!Array.isArray(entity.raw.slices)) return [];
  return entity.raw.slices.flatMap((value) => {
    const ref = record(value);
    const id = asId(ref?.id);
    return id.namespace && id.slug ? [{ kind: wireKind(ref?.kind) ?? 'slice', id }] : [];
  });
}

export function validationFixPrompt(
  model: Model,
  issue: Issue,
  options: { branch?: string; momentKey?: string; scopePath?: string } = {},
): string {
  const selected = new Map<string, Entity>();
  const referenced = new Map<string, ContextRef>();
  const refsByKey = new Map(model.entities.map((entity) => [entity.key, references(entity)]));
  const remember = (ref: ContextRef) => referenced.set(identity(ref), ref);
  const include = (entity: Entity) => {
    selected.set(entity.key, entity);
    for (const ref of refsByKey.get(entity.key) ?? []) remember(ref);
  };
  if (issue.subject) {
    remember(issue.subject);
    for (const entity of resolve(model, issue.subject)) include(entity);
  }
  const moment = options.momentKey ? model.byKey.get(options.momentKey) : undefined;
  if (moment) include(moment);

  const seeds = new Set([...selected.keys(), ...(issue.subject ? [identity(issue.subject)] : [])]);
  const touchesSeed = (entity: Entity) =>
    seeds.has(entity.key) || (refsByKey.get(entity.key) ?? []).some((ref) => seeds.has(identity(ref)));
  for (const slice of model.slices) if (touchesSeed(slice.entity)) include(slice.entity);

  const sourceEvents = new Set(
    [...selected.values()].flatMap((entity) =>
      (refsByKey.get(entity.key) ?? []).filter((ref) => ref.kind === 'event').map(identity),
    ),
  );
  for (const slice of model.slices) {
    if (slice.kind === 'commandSlice' && slice.events.some((event) => sourceEvents.has(entityKey('event', event.id)))) {
      include(slice.entity);
    }
  }

  const relevantKeys = new Set([...selected.keys(), ...seeds]);
  for (const storyboard of model.storyboards) {
    if (
      seeds.has(storyboard.key) ||
      touchesSeed(storyboard) ||
      storyboardSlices(storyboard).some((ref) =>
        ref.kind === 'slice'
          ? sliceKinds.some((kind) => relevantKeys.has(entityKey(kind, ref.id)))
          : relevantKeys.has(identity(ref)),
      )
    ) {
      include(storyboard);
      for (const ref of storyboardSlices(storyboard)) {
        remember(ref);
        for (const slice of resolve(model, ref)) include(slice);
      }
    }
  }

  if (issue.subject || moment) {
    for (let depth = 0; depth < 2; depth += 1) {
      const refs = [...referenced.values()];
      for (const ref of refs) for (const entity of resolve(model, ref)) include(entity);
    }
  } else {
    for (const entity of model.byKind.get('eventModel') ?? []) include(entity);
  }

  const missing = [...referenced.values()].filter((ref) => resolve(model, ref).length === 0).map(identity);
  const omitted = [...referenced.values()]
    .filter((ref) => resolve(model, ref).some((entity) => !selected.has(entity.key)))
    .map(identity);
  const subject = issue.subject ? model.byKey.get(identity(issue.subject)) : undefined;
  const definition = (entity: Entity) => ({ identity: entity.key, kind: entity.kind, id: entity.id, raw: entity.raw });
  const json = (value: unknown) => `\`\`\`json\n${JSON.stringify(value, null, 2)}\n\`\`\``;

  return [
    'Fix this validation finding in the Eventmodel project using the Eventmodel CLI or MCP tools.',
    'Re-read the current model and validation results in the specified branch and scope before editing. The snapshot below may be stale or incomplete.',
    'Fetch referenced definitions that are missing or omitted, including their exact pinned versions. Apply a targeted repair that preserves intended behavior, event causality, and existing contracts. Do not merely suppress the rule, downgrade its severity, or delete valid behavior to hide the finding.',
    'For temporal findings, inspect the full ordered storyboard, its slice definitions, and the source and emitted events before deciding whether to change ordering, references, or modeling. Do not assume an event missing from this snapshot does not exist.',
    'Apply the repair, rerun validation for the affected scope, and report the entities changed, the reason for the repair, and the validation results, including any remaining findings. If the model lacks enough information to choose a behavior-preserving repair, explain the specific ambiguity instead of inventing domain behavior.',
    '',
    '## Scope',
    json({
      branch: options.branch || 'baseline',
      scopePath: options.scopePath ?? null,
      momentKey: options.momentKey ?? null,
      namespaces: model.namespaces,
    }),
    '',
    '## Finding',
    json({
      code: issue.code,
      severity: issue.severity,
      message: issue.message,
      field: issue.field || null,
      subject: issue.subject ? { identity: identity(issue.subject), ...issue.subject } : null,
      rule: { title: issue.ruleTitle, category: issue.ruleCategory, defaultSeverity: issue.defaultSeverity ?? null },
    }),
    issue.subject
      ? subject
        ? 'The subject definition is included below.'
        : 'The subject is not loaded. Fetch its exact identity before proposing a repair.'
      : 'This is a model-wide finding with no attached subject. Re-read validation in this scope to identify the affected entities.',
    options.momentKey && !moment
      ? `The selected moment is not loaded: ${options.momentKey}. Fetch it if relevant.`
      : '',
    '',
    '## Relevant loaded storyboards',
    'Slice order and repeated occurrences are preserved exactly as loaded.',
    json(
      [...selected.values()]
        .filter((entity) => entity.kind === 'storyboard')
        .map((entity) => ({
          ...definition(entity),
          orderedSlices: storyboardSlices(entity).map(identity),
        })),
    ),
    '',
    '## Loaded subject and related definitions',
    json([...selected.values()].filter((entity) => entity.kind !== 'storyboard').map(definition)),
    '',
    '## References to fetch',
    json({ notLoaded: missing, loadedButOmitted: omitted }),
    'This is a focused snapshot, not the entire model. Follow relevant references in the raw definitions and fetch any additional current definitions needed for the repair. An empty reference list does not prove the model is complete.',
  ]
    .filter((line) => line !== '')
    .join('\n\n');
}

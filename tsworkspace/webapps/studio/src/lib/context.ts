// Markdown context builders for the "Copy context" action: a self-contained
// description of the selected entity or connection that can be pasted into an
// AI conversation.

import type { Issue, IssueIndex } from './issues';
import type { BoardEdgeData } from './layout';
import {
  contextSeams,
  type EdgeRef,
  type Entity,
  type FieldSpec,
  fieldTypeLabel,
  humanStatus,
  type Model,
  neighborsOf,
  neighborsOfMoment,
  type SliceView,
  storyboardEntryReadModel,
  trackingOf,
  upstreamContextsOf,
} from './model';

const fmtId = (id: { namespace: string; slug: string; version: string }) => `${id.namespace}/${id.slug}@v${id.version}`;

function fmtEdgeRef(label: string, e?: EdgeRef): string[] {
  if (!e) return [];
  return [`- ${label}: ${fmtId(e.id)}${e.doc ? `; ${e.doc}` : ''}`];
}

function fmtFields(fields: FieldSpec[]): string[] {
  return fields.map((f) => {
    const t = fieldTypeLabel(f);
    return `- ${String(f.name ?? '?')}${t ? `: ${t}` : ''}${f.doc ? `; ${String(f.doc)}` : ''}`;
  });
}

function sliceLines(s: SliceView): string[] {
  const head = `### ${s.entity.title} (${s.kind}, ${fmtId(s.entity.id)})`;
  const lines = [head];
  if (s.entity.doc) lines.push(s.entity.doc);
  lines.push(
    ...fmtEdgeRef('persona', s.persona),
    ...fmtEdgeRef('ui', s.ui),
    ...fmtEdgeRef('command', s.command),
    ...fmtEdgeRef('read model', s.readModel),
    ...s.readModels.flatMap((r) => fmtEdgeRef('source read model', r)),
    ...fmtEdgeRef('processor', s.processor),
    ...s.events.flatMap((e) => fmtEdgeRef(s.kind === 'commandSlice' ? 'emits event' : 'source event', e)),
  );
  return lines;
}

// The validator's findings, verbatim. A pasted context that omits them
// describes a broken model as if it were healthy, which is worse than
// saying nothing: the reader has no way to tell the two apart.
function fmtIssues(issues: Issue[]): string[] {
  if (issues.length === 0) return [];
  const lines = ['', '## Validation'];
  for (const i of issues) {
    const where = i.field ? ` (${i.field})` : '';
    // A rule that downgraded itself for this case reads as an
    // inconsistency unless the canonical severity is shown next to it.
    const downgraded =
      i.defaultSeverity && i.defaultSeverity !== i.severity ? ` [rule default: ${i.defaultSeverity}]` : '';
    lines.push(`- ${i.severity.toUpperCase()} ${i.code}${where}${downgraded}: ${i.message}`);
  }
  return lines;
}

export function entityContext(model: Model, entity: Entity, momentKey?: string, issues: Issue[] = []): string {
  const slug = `${entity.kind}:${entity.id.namespace}/${entity.id.slug}`;
  const touches = (kind: string, e?: EdgeRef) => Boolean(e) && `${kind}:${e?.id.namespace}/${e?.id.slug}` === slug;
  const memberships = model.slices.filter(
    (s) =>
      touches('persona', s.persona) ||
      touches('ui', s.ui) ||
      touches('command', s.command) ||
      touches('readModel', s.readModel) ||
      touches('processor', s.processor) ||
      s.events.some((e) => touches('event', e)) ||
      s.readModels.some((r) => touches('readModel', r)),
  );

  const lines: string[] = [
    'Context from an Event Modeling board (Event Model Studio).',
    '',
    `# ${entity.title}`,
    `- kind: ${entity.kind}`,
    `- id: ${fmtId(entity.id)}`,
  ];
  if (entity.uid) lines.push(`- uid: ${entity.uid}${entity.createdAt ? ` (created ${entity.createdAt})` : ''}`);
  if (entity.role) lines.push(`- role: ${entity.role}`);
  if (entity.swimlane) lines.push(`- stream: ${entity.swimlane.namespace}/${entity.swimlane.slug}`);
  if (entity.streamId) lines.push(`- stream identity: ${entity.streamId}`);
  for (const ns of upstreamContextsOf(entity))
    lines.push(
      `- translated from context: ${ns}; contexts touch only through published events into subscription views`,
    );
  for (const seam of contextSeams(model).filter(
    (x) =>
      entity.kind === 'event' &&
      x.upstreamEvent.namespace === entity.id.namespace &&
      x.upstreamEvent.slug === entity.id.slug,
  ))
    lines.push(`- feeds context: ${seam.view.id.namespace}; ${seam.view.title} (subscription view)`);
  const { backward, forward } = momentKey ? neighborsOfMoment(model, entity, momentKey) : neighborsOf(model, entity);
  for (const n of backward) lines.push(`- backward: ${n.entity.title}; ${n.relation}`);
  for (const n of forward) lines.push(`- forward: ${n.entity.title}; ${n.relation}`);
  for (const t of trackingOf(model, entity))
    lines.push(`- status: ${t.status || 'todo'} (tracker: ${t.tracker.title}${t.doc ? `; ${t.doc}` : ''})`);
  if (entity.kind === 'tracker')
    for (const raw of Array.isArray(entity.raw.items) ? (entity.raw.items as Record<string, unknown>[]) : []) {
      const subject = raw.subject as { kind?: string; id?: { slug?: string } } | undefined;
      lines.push(
        `- item: ${String(subject?.id?.slug ?? '?')}; ${humanStatus(raw.status) || 'todo'}${raw.doc ? ` (${String(raw.doc)})` : ''}`,
      );
    }
  if (entity.tech?.length) lines.push(`- tech: ${entity.tech.join(', ')}`);
  for (const c of entity.calls ?? [])
    lines.push(
      `- calls external system: ${c.namespace}/${c.slug}, outbound seam; results return via its webhook views`,
    );
  for (const t of Array.isArray(entity.raw.transitions) ? (entity.raw.transitions as Record<string, unknown>[]) : []) {
    const after = (t.after as { id?: { slug?: string } } | undefined)?.id?.slug ?? '?';
    const next = (Array.isArray(t.next) ? (t.next as Record<string, unknown>[]) : [])
      .map((n) => {
        const slug = (n.event as { id?: { slug?: string } } | undefined)?.id?.slug ?? '?';
        return `${slug}${n.doc ? ` (when ${String(n.doc)})` : ''}`;
      })
      .join(' | ');
    lines.push(`- lifecycle: after ${after} → ${next}; one successor wins per stream instance`);
  }
  if (entity.kind === 'readModel') {
    for (const c of model.continuations) {
      const rm = storyboardEntryReadModel(c.to);
      if (rm && rm.namespace === entity.id.namespace && rm.slug === entity.id.slug)
        lines.push(
          `- opens road: ${c.to.title}${c.doc ? `, when ${c.doc}` : ''} (observed state; the ${c.afterEvent.slug} stream decides)`,
        );
    }
  }
  for (const c of model.continuations.filter((c) => c.from.key === entity.key))
    lines.push(
      `- forks into: ${c.to.title}${c.doc ? `, when ${c.doc}` : ''} (alternative timeline, decided on the ${c.afterEvent.slug} stream state)`,
    );
  for (const c of model.continuations.filter((c) => c.to.key === entity.key))
    lines.push(`- continues from: ${c.from.title}${c.doc ? `, when ${c.doc}` : ''}`);
  if (entity.externalSource)
    lines.push(
      `- external source: ${entity.externalSource.system}${entity.externalSource.descriptor ? ` (${entity.externalSource.descriptor})` : ''}; populated from outside the system, not projected from internal events`,
    );
  lines.push(...fmtIssues(issues));
  if (entity.doc) lines.push('', '## Doc', entity.doc);
  if (entity.annotations.length > 0)
    lines.push('', '## Annotations', ...entity.annotations.map((a) => `- ${a.type}: ${JSON.stringify(a.data)}`));
  if (entity.fields.length > 0) lines.push('', '## Fields', ...fmtFields(entity.fields));
  const own = model.slices.find((s) => s.entity.key === entity.key);
  if (own && own.scenarios.length > 0) {
    lines.push('', '## Scenarios (Given / When / Then with example data)');
    for (const sc of own.scenarios) {
      lines.push('', `### ${sc.title || sc.id}${sc.doc ? `: ${sc.doc}` : ''}`);
      const ex = (e: { id: { namespace: string; slug: string }; data?: Record<string, unknown> }) =>
        `${e.id.slug}${e.data ? ` ${JSON.stringify(e.data)}` : ''}`;
      // An empty GIVEN is a claim, not an omission: this moment starts from
      // no prior state. Dropping the line makes "starts the stream" and
      // "nobody wrote the preconditions down" read identically. Read-model
      // slices have no precondition to state: their inputs are the WHEN.
      if (sc.given.length === 0 && own.kind !== 'readModelSlice') lines.push('- GIVEN nothing');
      for (const g of sc.given) lines.push(`- GIVEN ${ex(g)}`);
      if (sc.whenDoc) lines.push(`- WHEN ${sc.whenDoc}`);
      for (const w of sc.when) lines.push(`- WHEN ${ex(w)}`);
      if (sc.reject) lines.push(`- THEN REJECT ${sc.reject.reasonCode}${sc.reject.doc ? `: ${sc.reject.doc}` : ''}`);
      for (const t of sc.thenEmits) lines.push(`- THEN ${ex(t)}`);
      if (sc.thenState) lines.push(`- THEN state ${ex(sc.thenState)}`);
      if (sc.thenCommand) lines.push(`- THEN ${ex(sc.thenCommand)}`);
      for (const a of sc.annotations) lines.push(`- ${a.type}: ${JSON.stringify(a.data)}`);
    }
  }
  if (memberships.length > 0) {
    lines.push('', '## Slices touching this entity');
    for (const s of memberships) lines.push('', ...sliceLines(s));
  }
  lines.push('', '## Raw definition', '```json', JSON.stringify(entity.raw, null, 2), '```');
  return lines.join('\n');
}

export function entitiesContext(model: Model, entities: Entity[], issues?: IssueIndex): string {
  const parts = entities.map((e) => entityContext(model, e, undefined, issues?.for(e) ?? []));
  return [
    `Context bundle: ${entities.length} entities from an Event Modeling board.`,
    '',
    parts.join('\n\n---\n\n'),
  ].join('\n');
}

export function edgeContext(edge: BoardEdgeData): string {
  const endpoint = (label: string, e: Entity): string[] => [
    '',
    `## ${label}: ${e.title}`,
    `- kind: ${e.kind}`,
    `- id: ${fmtId(e.id)}`,
    ...(e.doc ? [`- doc: ${e.doc}`] : []),
  ];
  const lines: string[] = [
    'Context from an Event Modeling board (Event Model Studio).',
    '',
    `# Connection: ${edge.source.title} → ${edge.target.title}`,
    `- relation: ${edge.source.kind} ${edge.relation || '→'} ${edge.target.kind}`,
    ...endpoint('Source', edge.source),
    ...endpoint('Target', edge.target),
  ];
  if (edge.doc) lines.push('', '## Connection doc', edge.doc);
  if (edge.metadata.length > 0)
    lines.push('', '## Connection metadata', '```json', JSON.stringify(edge.metadata, null, 2), '```');
  if (edge.slice)
    lines.push(
      '',
      `## Declared by slice: ${edge.slice.title} (${edge.slice.kind})`,
      '```json',
      JSON.stringify(edge.slice.raw, null, 2),
      '```',
    );
  return lines.join('\n');
}

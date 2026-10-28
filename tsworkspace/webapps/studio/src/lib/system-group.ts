import type { Entity, EntityKind } from '@/lib/model';

export interface GroupTarget {
  kind: EntityKind | undefined;
  wire: string;
  id: { namespace: string; slug: string; version: string } | undefined;
}

export interface GroupEdge {
  section: 'members' | 'relationships' | 'actors';
  path: string;
  connectionId: string;
  target: GroupTarget;
  direction?: string;
  label?: string;
  doc?: string;
}

const TARGET_KINDS: Record<string, EntityKind> = {
  systemGroup: 'systemGroup',
  component: 'component',
  externalSystem: 'externalSystem',
  dataStore: 'dataStore',
  schema: 'schema',
  domain: 'domain',
  dataProduct: 'dataProduct',
  persona: 'persona',
};

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function text(value: unknown): string | undefined {
  const inner = record(value).value;
  return typeof inner === 'string' ? inner : undefined;
}

function identity(value: unknown): GroupTarget['id'] {
  const id = record(record(value).id);
  if (typeof id.namespace !== 'string' || typeof id.slug !== 'string') return undefined;
  return { namespace: id.namespace, slug: id.slug, version: String(id.version ?? '') };
}

function oneOf(value: unknown): GroupTarget {
  const [wire, reference] = Object.entries(record(value)).find(([, inner]) => inner && typeof inner === 'object') ?? [
    '',
    undefined,
  ];
  return { kind: TARGET_KINDS[wire], wire, id: identity(reference) };
}

function edges(
  section: GroupEdge['section'],
  entries: unknown,
  target: (entry: Record<string, unknown>) => GroupTarget,
): GroupEdge[] {
  if (!Array.isArray(entries)) return [];
  return entries.map((value, index) => {
    const entry = record(value);
    return {
      section,
      path: `${section}[${index}]`,
      connectionId: text(entry.connectionId) ?? '',
      target: target(entry),
      direction: typeof entry.direction === 'string' ? entry.direction : undefined,
      label: text(entry.label),
      doc: text(entry.doc),
    };
  });
}

export function groupEdges(entity: Entity): GroupEdge[] {
  if (entity.kind === 'systemGroup') {
    return [
      ...edges('members', entity.raw.members, (entry) => oneOf(entry.member)),
      ...edges('relationships', entity.raw.relationships, (entry) => ({
        kind: 'systemGroup',
        wire: 'systemGroup',
        id: identity(entry.target),
      })),
      ...edges('actors', entity.raw.actors, (entry) => oneOf(entry.target)),
    ];
  }
  if (entity.kind === 'domain') return edges('members', entity.raw.members, (entry) => oneOf(entry.member));
  return [];
}

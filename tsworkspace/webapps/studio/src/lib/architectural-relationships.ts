import { asId, entityKey, type Entity, type EntityId, type Model, wireKind } from './model';
import type { ResponsibilitySubject } from './responsibility';

export interface ArchitecturalDeclaration {
  owner: Entity;
  path: string;
  connectionId?: string;
  target?: { kind: 'component' | 'externalSystem'; id: EntityId };
  label: string;
  doc?: string;
}
function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}
function text(value: unknown): string | undefined {
  const raw = record(value).value;
  return typeof raw === 'string' ? raw : undefined;
}
export function architecturalDeclarations(owner: Entity): ArchitecturalDeclaration[] {
  if (owner.kind !== 'component' && owner.kind !== 'externalSystem') return [];
  if (!Array.isArray(owner.raw.relationships)) return [];
  return owner.raw.relationships.map((value, index) => {
    const body = record(value);
    const endpoint = record(body.target);
    const alternatives = (['component', 'externalSystem'] as const).filter((kind) => endpoint[kind]);
    const kind = alternatives.length === 1 ? alternatives[0] : undefined;
    return {
      owner,
      path: `relationships[${index}]`,
      connectionId: text(body.connectionId),
      target: kind ? { kind, id: asId(record(endpoint[kind]).id) } : undefined,
      label: text(body.label) ?? 'Undeclared label',
      doc: text(body.doc),
    };
  });
}
export function incomingArchitecturalDeclarations(model: Model, target: Entity): ArchitecturalDeclaration[] {
  return model.entities
    .flatMap(architecturalDeclarations)
    .filter(
      (declaration) => declaration.target && entityKey(declaration.target.kind, declaration.target.id) === target.key,
    );
}
export function resolveArchitecturalSubject(
  model: Model,
  subject: ResponsibilitySubject,
): ArchitecturalDeclaration | undefined {
  const connection = record(record(subject).connection);
  const owner = record(connection.owner);
  const ownerUid = connection.ownerUid;
  const id = text(connection.id);
  if (typeof ownerUid !== 'string' || !ownerUid || !id || typeof owner.kind !== 'string') return undefined;
  const kind = wireKind(owner.kind);
  const entity = kind ? model.byKey.get(entityKey(kind, asId(owner.id))) : undefined;
  if (!entity || entity.uid !== ownerUid) return undefined;
  const declarations = architecturalDeclarations(entity);
  const ids = [
    ...declarations.map((item) => item.connectionId),
    ...(Array.isArray(entity.raw.communications)
      ? entity.raw.communications.map((item) => text(record(item).connectionId))
      : []),
  ];
  if (ids.some((value) => !value) || new Set(ids).size !== ids.length) return undefined;
  return declarations.find((item) => item.connectionId === id);
}

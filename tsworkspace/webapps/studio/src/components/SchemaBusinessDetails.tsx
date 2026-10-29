import type { Entity, Model } from '@/lib/model';

type RawSegment = { name?: string; repeated?: boolean };
type RawPath = { segments?: RawSegment[] };
type RawId = { namespace?: string; slug?: string; version?: string | number };
type RawField = {
  name?: string;
  repeated?: boolean;
  type?: {
    object?: { fields?: RawField[] };
    ref?: { ref?: { schema?: { id?: RawId } } };
  };
};

const MAX_SEGMENTS = 32;
const MAX_SEGMENT_BYTES = 256;

function fieldsOf(raw: Record<string, unknown>): RawField[] {
  return Array.isArray(raw.fields) ? (raw.fields as RawField[]) : [];
}

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).length;
}

/** Loaded-field walk used only to label what this model can see. */
function pathResolves(fields: RawField[], path: RawPath | undefined): boolean {
  const segments = path?.segments ?? [];
  if (segments.length === 0 || segments.length > MAX_SEGMENTS) return false;
  let current = fields;
  for (let index = 0; index < segments.length; index += 1) {
    const segment = segments[index];
    const name = segment.name ?? '';
    if (name.length === 0 || utf8Length(name) > MAX_SEGMENT_BYTES) return false;
    const matches = current.filter((field) => field.name === name);
    if (matches.length !== 1) return false;
    const field = matches[0];
    if ((segment.repeated === true) !== (field.repeated === true)) return false;
    const nested = field.type?.object?.fields;
    if (index < segments.length - 1) {
      if (!nested) return false;
      current = nested;
    }
  }
  return true;
}

function pathLabel(path: RawPath | undefined): string {
  const segments = path?.segments ?? [];
  if (segments.length === 0) return 'empty path';
  return segments.map((segment) => `${segment.name ?? ''}${segment.repeated ? ' (repeated)' : ''}`).join(' / ');
}

function schemaIdLabel(id: RawId | undefined): string {
  if (!id) return 'unset schema';
  return `${id.namespace ?? ''}/${id.slug ?? ''}@${String(id.version ?? '')}`;
}

function roleLabel(role: unknown): string {
  if (role === 1 || role === 'BUSINESS_IDENTIFIER_ROLE_PRIMARY') return 'primary';
  if (role === 2 || role === 'BUSINESS_IDENTIFIER_ROLE_ALTERNATE') return 'alternate';
  return 'unspecified role';
}

function connectionLabel(value: unknown): string {
  if (!value || typeof value !== 'object' || !('value' in value)) return 'omitted';
  const id = (value as { value?: unknown }).value;
  return typeof id === 'string' && id.length > 0 ? id : 'omitted';
}

function typeReferences(fields: RawField[], prefix = 'fields'): { path: string; target: string }[] {
  const found: { path: string; target: string }[] = [];
  fields.forEach((field, index) => {
    const path = `${prefix}[${index}]`;
    const target = field.type?.ref?.ref?.schema?.id;
    if (target) found.push({ path: `${path}.ref`, target: schemaIdLabel(target) });
    const nested = field.type?.object?.fields;
    if (nested) found.push(...typeReferences(nested, `${path}.object.fields`));
  });
  return found;
}

function loadedSchema(model: Model, id: RawId | undefined): Entity | undefined {
  if (!id?.namespace || !id.slug || id.version === undefined || id.version === '') return undefined;
  const version = String(id.version);
  return model.entities.find(
    (entity) =>
      entity.kind === 'schema' &&
      entity.id.namespace === id.namespace &&
      entity.id.slug === id.slug &&
      entity.id.version === version,
  );
}

function aggregateLabel(raw: Record<string, unknown>): string {
  if (raw.aggregateRoot === true) return 'true';
  if (raw.aggregateRoot === false) return 'false';
  return 'unspecified';
}

export function SchemaBusinessDetails({ entity, model }: { entity: Entity; model: Model }) {
  if (entity.kind !== 'schema') return null;
  const fields = fieldsOf(entity.raw);
  const identifiers = Array.isArray(entity.raw.identifiers) ? entity.raw.identifiers : [];
  const relationships = Array.isArray(entity.raw.propertyRelationships) ? entity.raw.propertyRelationships : [];
  const refs = typeReferences(fields);
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">Schema business declarations</h3>
      <p>Loaded-model inspection. This is not the backend reference check.</p>
      <p>
        Aggregate root intent: <span>{aggregateLabel(entity.raw)}</span>
      </p>
      <h4 className="font-semibold">Identifiers</h4>
      {identifiers.length === 0 && <p>No declared identifier role.</p>}
      {identifiers.map((value, index) => {
        const item = (value ?? {}) as {
          connectionId?: unknown;
          property?: RawPath;
          role?: unknown;
        };
        const resolved = pathResolves(fields, item.property);
        return (
          <section key={`identifier-${index}`} className="flex flex-col gap-1 rounded border p-2">
            <p>{roleLabel(item.role)}</p>
            <p>{pathLabel(item.property)}</p>
            <p>{connectionLabel(item.connectionId)}</p>
            <p>{resolved ? 'path resolves in this loaded schema' : 'path does not resolve in this loaded schema'}</p>
          </section>
        );
      })}
      <h4 className="font-semibold">Field type references</h4>
      <p>A field type reference and a property relationship are separate facts.</p>
      {refs.length === 0 && <p>No field type reference in this loaded schema.</p>}
      {refs.map((reference) => (
        <p key={reference.path}>
          {reference.path} {reference.target}
        </p>
      ))}
      <h4 className="font-semibold">Property relationships</h4>
      {relationships.length === 0 && <p>No property relationship in this loaded schema.</p>}
      {relationships.map((value, index) => {
        const item = (value ?? {}) as {
          connectionId?: unknown;
          source?: RawPath;
          targetEntity?: { id?: RawId };
          targetProperty?: RawPath;
          label?: { value?: string };
        };
        const sourceResolves = pathResolves(fields, item.source);
        const target = loadedSchema(model, item.targetEntity?.id);
        let targetInspection = 'entity only';
        if (item.targetProperty) {
          if (!target) targetInspection = 'target schema is not in the loaded model';
          else if (pathResolves(fieldsOf(target.raw), item.targetProperty)) {
            targetInspection = 'target path resolves in the loaded model';
          } else targetInspection = 'target path is unresolved in the loaded model';
        }
        return (
          <section key={`relationship-${index}`} className="flex flex-col gap-1 rounded border p-2">
            <p>{pathLabel(item.source)}</p>
            <p>
              {sourceResolves
                ? 'source path resolves in this loaded schema'
                : 'source path does not resolve in this loaded schema'}
            </p>
            <p>{schemaIdLabel(item.targetEntity?.id)}</p>
            <p>{item.label?.value && item.label.value.trim() ? item.label.value : 'no label'}</p>
            <p>{item.targetProperty ? pathLabel(item.targetProperty) : 'no target property'}</p>
            <p>{targetInspection}</p>
            <p>{connectionLabel(item.connectionId)}</p>
          </section>
        );
      })}
    </section>
  );
}

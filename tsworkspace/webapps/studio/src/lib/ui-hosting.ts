import type { Entity, EntityId, Model } from './model';
import { entityKey, wireKind } from './model';
import { hasUnambiguousOwnerIdentities, nativeEntityRef } from './responsibility';
import { contractRecord as record } from './specification';

export interface HostingConnection {
  owner: { kind: string; id: EntityId };
  ownerUid: string;
  id: { value: string };
}
export interface UiHostingView {
  owner: Entity;
  path: string;
  connectionId?: string;
  ui?: { id: EntityId };
  label?: string;
  doc?: string;
}
const text = (value: unknown): string | undefined => {
  const item = record(value).value;
  return typeof item === 'string' ? item : undefined;
};
function exactId(value: unknown): EntityId | undefined {
  const body = record(value);
  if (typeof body.namespace !== 'string' || !body.namespace || typeof body.slug !== 'string' || !body.slug)
    return undefined;
  const version =
    typeof body.version === 'string'
      ? body.version
      : typeof body.version === 'number' && Number.isSafeInteger(body.version)
        ? String(body.version)
        : '';
  if (!/^[1-9][0-9]*$/.test(version) || BigInt(version) > 18_446_744_073_709_551_615n) return undefined;
  return { namespace: body.namespace, slug: body.slug, version };
}
export function uiHostings(owner: Entity): UiHostingView[] {
  if (owner.kind !== 'component' || !Array.isArray(owner.raw.uiHostings)) return [];
  return owner.raw.uiHostings.map((value, index) => {
    const body = record(value);
    const ui = exactId(record(body.ui).id);
    return {
      owner,
      path: `uiHostings[${index}]`,
      connectionId: text(body.connectionId),
      ui: ui ? { id: ui } : undefined,
      label: text(body.label),
      doc: text(body.doc),
    };
  });
}
export function hostingConnection(item: UiHostingView): HostingConnection | undefined {
  const ownerId = exactId(item.owner.raw.id);
  if (
    !ownerId ||
    entityKey('component', ownerId) !== item.owner.key ||
    !item.owner.uid ||
    !item.connectionId ||
    !hasUnambiguousOwnerIdentities(item.owner)
  )
    return undefined;
  return { owner: nativeEntityRef(item.owner), ownerUid: item.owner.uid, id: { value: item.connectionId } };
}
export function resolveUiHostingSubject(model: Model, subject: unknown): UiHostingView | undefined {
  const selected = record(record(subject).connection);
  const owner = record(selected.owner);
  const kind = typeof owner.kind === 'string' ? wireKind(owner.kind) : undefined;
  if (kind !== 'component' || typeof selected.ownerUid !== 'string' || !selected.ownerUid) return undefined;
  const ownerId = exactId(owner.id);
  if (!ownerId) return undefined;
  const entity = model.byKey.get(entityKey(kind, ownerId));
  if (!entity || entity.uid !== selected.ownerUid || !hasUnambiguousOwnerIdentities(entity)) return undefined;
  return uiHostings(entity).find((item) => item.connectionId && item.connectionId === text(selected.id));
}

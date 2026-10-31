import type { Entity, EntityId, Model } from './model';
import { asId, entityKey, wireKind } from './model';
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
export function uiHostings(owner: Entity): UiHostingView[] {
  if (owner.kind !== 'component' || !Array.isArray(owner.raw.uiHostings)) return [];
  return owner.raw.uiHostings.map((value, index) => {
    const body = record(value);
    const ui = record(body.ui);
    return {
      owner,
      path: `uiHostings[${index}]`,
      connectionId: text(body.connectionId),
      ui: ui.id ? { id: asId(ui.id) } : undefined,
      label: text(body.label),
      doc: text(body.doc),
    };
  });
}
export function hostingConnection(item: UiHostingView): HostingConnection | undefined {
  if (!item.owner.uid || !item.connectionId || !hasUnambiguousOwnerIdentities(item.owner)) return undefined;
  return { owner: nativeEntityRef(item.owner), ownerUid: item.owner.uid, id: { value: item.connectionId } };
}
export function resolveUiHostingSubject(model: Model, subject: unknown): UiHostingView | undefined {
  const selected = record(record(subject).connection);
  const owner = record(selected.owner);
  const kind = typeof owner.kind === 'string' ? wireKind(owner.kind) : undefined;
  if (kind !== 'component' || typeof selected.ownerUid !== 'string' || !selected.ownerUid) return undefined;
  const entity = model.byKey.get(entityKey(kind, asId(owner.id)));
  if (!entity || entity.uid !== selected.ownerUid || !hasUnambiguousOwnerIdentities(entity)) return undefined;
  return uiHostings(entity).find((item) => item.connectionId && item.connectionId === text(selected.id));
}

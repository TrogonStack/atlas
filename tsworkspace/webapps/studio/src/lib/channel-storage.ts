import { asId, entityKey, type Entity, type EntityId, type Model, wireKind } from './model';
import { contractRecord as record } from './specification';
import { hasUnambiguousOwnerIdentities } from './responsibility';

export interface ChannelLinkView {
  owner: Entity;
  path: string;
  connectionId?: string;
  kind: 'COMPOSES' | 'ROUTES_TO' | 'Unavailable';
  target?: { id: EntityId };
  parameterValues: [string, string][];
  doc?: string;
}
export interface StorageClaimView {
  owner: Entity;
  path: string;
  connectionId?: string;
  direction: 'READ' | 'WRITE' | 'Unavailable';
  store?: { id: EntityId };
  attachment?: {
    owner: Record<string, unknown>;
    ownerUid?: string;
    id?: string;
  };
  portId?: string;
  doc?: string;
}
export type AssociationView = ChannelLinkView | StorageClaimView;

const text = (value: unknown): string | undefined => {
  const raw = record(value).value;
  return typeof raw === 'string' ? raw : undefined;
};
const present = (value: unknown) => (record(value).id ? { id: asId(record(value).id) } : undefined);

export function channelLinks(owner: Entity): ChannelLinkView[] {
  if (owner.kind !== 'channel' || !Array.isArray(owner.raw.links)) return [];
  return owner.raw.links.map((value, index) => {
    const body = record(value);
    return {
      owner,
      path: `links[${index}]`,
      connectionId: text(body.connectionId),
      kind:
        body.kind === 'CHANNEL_LINK_KIND_COMPOSES'
          ? 'COMPOSES'
          : body.kind === 'CHANNEL_LINK_KIND_ROUTES_TO'
            ? 'ROUTES_TO'
            : 'Unavailable',
      target: present(body.target),
      parameterValues: Object.entries(record(body.parameterValues)).map(([name, item]) => [name, String(item)]),
      doc: text(body.doc),
    };
  });
}
export function storageClaims(owner: Entity): StorageClaimView[] {
  if (!['component', 'externalSystem', 'dataProduct'].includes(owner.kind) || !Array.isArray(owner.raw.storageClaims))
    return [];
  return owner.raw.storageClaims.map((value, index) => {
    const body = record(value);
    const attachment = body.attachment === undefined ? undefined : record(body.attachment);
    return {
      owner,
      path: `storageClaims[${index}]`,
      connectionId: text(body.connectionId),
      direction:
        body.direction === 'STORAGE_DIRECTION_READ'
          ? 'READ'
          : body.direction === 'STORAGE_DIRECTION_WRITE'
            ? 'WRITE'
            : 'Unavailable',
      store: present(body.store),
      attachment: attachment && {
        owner: record(attachment.owner),
        ownerUid: typeof attachment.ownerUid === 'string' ? attachment.ownerUid : undefined,
        id: text(attachment.id),
      },
      portId: text(body.portId),
      doc: text(body.doc),
    };
  });
}
export function incomingChannelLinks(model: Model, target: Entity): ChannelLinkView[] {
  return model.entities
    .flatMap(channelLinks)
    .filter((item) => item.target && entityKey('channel', item.target.id) === target.key);
}
export function incomingStorageClaims(model: Model, target: Entity): StorageClaimView[] {
  return model.entities
    .flatMap(storageClaims)
    .filter((item) => item.store && entityKey('dataStore', item.store.id) === target.key);
}
export function resolveAssociationSubject(model: Model, subject: unknown): AssociationView | undefined {
  const connection = record(record(subject).connection);
  const owner = record(connection.owner);
  const kind = typeof owner.kind === 'string' ? wireKind(owner.kind) : undefined;
  const id = text(connection.id);
  if (!kind || !id || typeof connection.ownerUid !== 'string' || !connection.ownerUid) return undefined;
  const entity = model.byKey.get(entityKey(kind, asId(owner.id)));
  if (!entity || entity.uid !== connection.ownerUid) return undefined;
  if (!hasUnambiguousOwnerIdentities(entity)) return undefined;
  return [...channelLinks(entity), ...storageClaims(entity)].find((item) => item.connectionId === id);
}
export function resolvedAttachment(model: Model, item: StorageClaimView): Entity | undefined {
  const attachment = item.attachment;
  const kind = typeof attachment?.owner.kind === 'string' ? wireKind(attachment.owner.kind) : undefined;
  if (!attachment || kind !== 'dataStore' || !attachment.id || !attachment.ownerUid) return undefined;
  const store = model.byKey.get(entityKey('dataStore', asId(attachment.owner.id)));
  if (!store || store.uid !== attachment.ownerUid || !hasUnambiguousOwnerIdentities(store)) return undefined;
  const members = Array.isArray(store.raw.attachments) ? store.raw.attachments : [];
  return members.some((member) => text(record(member).connectionId) === attachment.id) ? store : undefined;
}

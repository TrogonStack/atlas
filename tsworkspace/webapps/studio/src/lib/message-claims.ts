import { asId, entityKey, type Entity, type EntityKind, type Model, wireKind } from './model';
import { contractMember, contractRecord as record, contractValue } from './specification';

export const claimLists = ['claims', 'routes', 'triggers', 'fieldSelections'] as const;
export type ClaimList = (typeof claimLists)[number];

export interface MessageClaimLeaf {
  owner: Entity;
  list: ClaimList;
  path: string;
  connectionId?: string;
  body: Record<string, unknown>;
}

const text = (value: unknown): string | undefined => {
  const raw = record(value).value;
  return typeof raw === 'string' ? raw : undefined;
};
const claimOwner = (entity: Entity) =>
  entity.kind === 'component' || entity.kind === 'externalSystem' || entity.kind === 'domain';
const listOf = (owner: Entity, list: ClaimList): unknown[] => {
  const value = record(owner.raw.messageClaims)[list];
  return Array.isArray(value) ? value : [];
};

export function messageClaimLeaves(owner: Entity): MessageClaimLeaf[] {
  if (!claimOwner(owner)) return [];
  return claimLists.flatMap((list) =>
    listOf(owner, list).map((value, index) => {
      const body = record(value);
      return { owner, list, path: `messageClaims.${list}[${index}]`, connectionId: text(body.connectionId), body };
    }),
  );
}

export function identityInventory(entity: Entity): (string | undefined)[] {
  const ids = ['relationships', 'communications'].flatMap((field) =>
    Array.isArray(entity.raw[field]) ? entity.raw[field].map((item) => text(record(item).connectionId)) : [],
  );
  return [...ids, ...messageClaimLeaves(entity).map((leaf) => leaf.connectionId)];
}

export function hasUniqueIdentities(entity: Entity): boolean {
  const ids = identityInventory(entity);
  return !ids.some((value) => !value) && new Set(ids).size === ids.length;
}

export function resolveMessageClaimSubject(model: Model, subject: unknown): MessageClaimLeaf | undefined {
  const connection = record(record(subject).connection);
  const owner = record(connection.owner);
  const kind = typeof owner.kind === 'string' ? wireKind(owner.kind) : undefined;
  const id = text(connection.id);
  if (!kind || !id || typeof connection.ownerUid !== 'string' || !connection.ownerUid) return undefined;
  const entity = model.byKey.get(entityKey(kind, asId(owner.id)));
  if (!entity || entity.uid !== connection.ownerUid || !hasUniqueIdentities(entity)) return undefined;
  return messageClaimLeaves(entity).find((leaf) => leaf.connectionId === id);
}

export function claimTarget(leaf: MessageClaimLeaf): { kind: EntityKind; entityKey: string } | undefined {
  const { body } = leaf;
  const reference = (kind: EntityKind, value: unknown) => {
    const id = record(value).id;
    return id ? { kind, entityKey: entityKey(kind, asId(id)) } : undefined;
  };
  if (leaf.list === 'claims' || leaf.list === 'triggers')
    return reference('specification', record(body.message).specification);
  if (leaf.list === 'routes') return reference('channel', body.channel);
  const owner = record(record(body.binding).owner);
  return owner.kind === 'ENTITY_KIND_SPECIFICATION' ? reference('specification', owner) : undefined;
}

export function incomingMessageClaims(model: Model, target: Entity): MessageClaimLeaf[] {
  return model.entities.flatMap(messageClaimLeaves).filter((leaf) => claimTarget(leaf)?.entityKey === target.key);
}

export function resolveMessageMember(model: Model, selection: Record<string, unknown>) {
  const owner = model.byKey.get(entityKey('specification', asId(record(selection.specification).id)));
  if (
    !owner ||
    typeof selection.ownerUid !== 'string' ||
    !selection.ownerUid ||
    owner.uid !== selection.ownerUid ||
    selection.kind !== 'CONTRACT_MEMBER_KIND_MESSAGE' ||
    selection.expectedDigest !== undefined
  )
    return undefined;
  const member = contractMember(owner, 'messages', selection.memberId);
  return member ? { owner, name: contractValue(member.name) } : undefined;
}

export function resolvePayloadBinding(model: Model, binding: Record<string, unknown>): boolean {
  const owner = record(binding.owner);
  const kind = typeof owner.kind === 'string' ? wireKind(owner.kind) : undefined;
  const id = text(binding.id);
  const entity = kind === 'specification' ? model.byKey.get(entityKey(kind, asId(owner.id))) : undefined;
  if (!entity || !id || typeof binding.ownerUid !== 'string' || !binding.ownerUid || entity.uid !== binding.ownerUid)
    return false;
  const found = (Array.isArray(entity.raw.messages) ? entity.raw.messages : []).flatMap((message) => {
    const schemas = record(message).schemas;
    return Array.isArray(schemas) ? schemas.map(record) : [];
  });
  const matches = found.filter((schema) => text(schema.connectionId) === id);
  return matches.length === 1 && matches[0].role === 'ROLE_PAYLOAD';
}

export function localClaim(leaf: MessageClaimLeaf): MessageClaimLeaf | 'ambiguous' | undefined {
  const id = text(leaf.body.claimId);
  if (!id) return undefined;
  const matches = messageClaimLeaves(leaf.owner).filter((item) => item.list === 'claims' && item.connectionId === id);
  return matches.length > 1 ? 'ambiguous' : matches[0];
}

export type DisplaySelector =
  | { kind: 'path'; text: string }
  | { kind: 'opaque'; text: string }
  | { kind: 'unavailable' };

export function displaySelector(value: unknown): DisplaySelector {
  const body = record(value);
  const path = record(body.path).segments;
  if (Array.isArray(path) && !body.opaque && path.every((segment) => typeof segment === 'string'))
    return { kind: 'path', text: JSON.stringify(path) };
  const opaque = record(body.opaque);
  if (body.opaque && !body.path && typeof opaque.syntax === 'string' && typeof opaque.value === 'string')
    return { kind: 'opaque', text: `${opaque.syntax}: ${opaque.value}` };
  return { kind: 'unavailable' };
}

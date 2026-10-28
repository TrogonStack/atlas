import { asId, entityKey, type Entity, type Model, wireKind } from './model';
import { contractMember, contractRecord as record, contractValue } from './specification';
import { hasUnambiguousOwnerIdentities } from './responsibility';

export interface CommunicationIntent {
  owner: Entity;
  path: string;
  connectionId?: string;
  direction: 'SEND' | 'RECEIVE' | 'Unavailable';
  peer?: { kind: 'component' | 'externalSystem'; id: ReturnType<typeof asId> };
  message: Record<string, unknown>;
  label: string;
  condition?: string;
  doc?: string;
}
const text = (value: unknown): string | undefined => {
  const raw = record(value).value;
  return typeof raw === 'string' ? raw : undefined;
};
export function communications(owner: Entity): CommunicationIntent[] {
  if (owner.kind !== 'component' && owner.kind !== 'externalSystem') return [];
  if (!Array.isArray(owner.raw.communications)) return [];
  return owner.raw.communications.map((value, index) => {
    const body = record(value);
    const endpoint = record(body.peer);
    const choices = (['component', 'externalSystem'] as const).filter((kind) => endpoint[kind]);
    const kind = choices.length === 1 ? choices[0] : undefined;
    return {
      owner,
      path: `communications[${index}]`,
      connectionId: text(body.connectionId),
      direction:
        body.direction === 'COMMUNICATION_DIRECTION_SEND'
          ? 'SEND'
          : body.direction === 'COMMUNICATION_DIRECTION_RECEIVE'
            ? 'RECEIVE'
            : 'Unavailable',
      peer: kind ? { kind, id: asId(record(endpoint[kind]).id) } : undefined,
      message: record(body.message),
      label: text(body.label) ?? 'Undeclared label',
      condition: text(body.condition),
      doc: text(body.doc),
    };
  });
}
export function incomingCommunications(model: Model, target: Entity): CommunicationIntent[] {
  return model.entities
    .flatMap(communications)
    .filter((item) => item.peer && entityKey(item.peer.kind, item.peer.id) === target.key);
}
export function resolveCommunicationSubject(model: Model, subject: unknown): CommunicationIntent | undefined {
  const connection = record(record(subject).connection);
  const owner = record(connection.owner);
  const kind = typeof owner.kind === 'string' ? wireKind(owner.kind) : undefined;
  const id = text(connection.id);
  if (!kind || !id || typeof connection.ownerUid !== 'string' || !connection.ownerUid) return undefined;
  const entity = model.byKey.get(entityKey(kind, asId(owner.id)));
  if (!entity || entity.uid !== connection.ownerUid) return undefined;
  if (!hasUnambiguousOwnerIdentities(entity)) return undefined;
  return communications(entity).find((item) => item.connectionId === id);
}
export function resolvedCommunicationMessage(
  model: Model,
  item: CommunicationIntent,
): { owner: Entity; name: string } | undefined {
  const selection = item.message;
  const owner = model.byKey.get(entityKey('specification', asId(record(selection.specification).id)));
  if (
    !owner ||
    !selection.ownerUid ||
    owner.uid !== selection.ownerUid ||
    selection.kind !== 'CONTRACT_MEMBER_KIND_MESSAGE' ||
    selection.expectedDigest !== undefined
  )
    return undefined;
  const member = contractMember(owner, 'messages', selection.memberId);
  return member ? { owner, name: contractValue(member.name) } : undefined;
}

import type { CatalogCheckedOwner, CatalogInspection } from './decision-document';
import type { Entity } from './model';
import { hasUnambiguousOwnerIdentities, nativeEntityRef } from './responsibility';
import { contractRecord as record } from './specification';
import type { HostingConnection } from './ui-hosting';

export type HumanPathType = 'issuance' | 'display';
export type HumanInteractionRequest = { hosting: HostingConnection } & (
  | { issuance: { persona: HostingConnection; ui: HostingConnection; command: HostingConnection } }
  | { display: { persona: HostingConnection; ui: HostingConnection; sourceReadModel: HostingConnection } }
);
export interface HumanInteractionResponse {
  request?: HumanInteractionRequest;
  state?: string;
  consistency?: string;
  scope?: CatalogInspection['scope'];
  edges?: {
    role?: string;
    selection?: HostingConnection;
    field?: string;
    target?: CatalogCheckedOwner['owner'];
    state?: string;
    checkedTarget?: CatalogCheckedOwner;
  }[];
}
export interface SliceOccurrence {
  path: string;
  connection: HostingConnection;
}
export function sliceOccurrences(
  owner: Entity,
  field: 'persona' | 'ui' | 'command' | 'sourceReadModels',
): SliceOccurrence[] {
  const ownerUid = owner.uid;
  if (!ownerUid || !hasUnambiguousOwnerIdentities(owner)) return [];
  if (owner.kind !== 'commandSlice' && owner.kind !== 'uiSlice') return [];
  if (
    (field === 'command' && owner.kind !== 'commandSlice') ||
    (field === 'sourceReadModels' && owner.kind !== 'uiSlice')
  )
    return [];
  const authored = owner.raw[field];
  const values = Array.isArray(authored)
    ? authored.map((value, index) => ({ value, path: `${field}[${index}]` }))
    : authored === undefined
      ? []
      : [{ value: authored, path: field }];
  return values.flatMap(({ value, path }) => {
    const id = record(record(value).connectionId).value;
    return typeof id === 'string' && id
      ? [{ path, connection: { owner: nativeEntityRef(owner), ownerUid, id: { value: id } } }]
      : [];
  });
}

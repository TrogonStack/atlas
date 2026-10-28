import type { Entity, Model } from './model';

export interface ChannelLinkView {
  owner: Entity;
  path: string;
  connectionId?: string;
  kind: 'COMPOSES' | 'ROUTES_TO' | 'Unavailable';
  target?: { id: { namespace: string; slug: string; version: string } };
  parameterValues: [string, string][];
  doc?: string;
}
export interface StorageClaimView {
  owner: Entity;
  path: string;
  connectionId?: string;
  direction: 'READ' | 'WRITE' | 'Unavailable';
  store?: { id: { namespace: string; slug: string; version: string } };
  attachment?: { owner: Record<string, unknown>; ownerUid?: string; id?: string };
  portId?: string;
  doc?: string;
}
export type AssociationView = ChannelLinkView | StorageClaimView;

export const channelLinks = (_owner: Entity): ChannelLinkView[] => [];
export const storageClaims = (_owner: Entity): StorageClaimView[] => [];
export const incomingChannelLinks = (_model: Model, _target: Entity): ChannelLinkView[] => [];
export const incomingStorageClaims = (_model: Model, _target: Entity): StorageClaimView[] => [];
export const resolveAssociationSubject = (_model: Model, _subject: unknown): AssociationView | undefined => undefined;

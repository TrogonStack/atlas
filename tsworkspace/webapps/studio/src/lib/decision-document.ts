import type { EntityId } from './model';
import type { NativeEntityRef, ResponsibilitySubject } from './responsibility';

export type CatalogSubject = ResponsibilitySubject;
export type DocumentSelection =
  | { document: { id: EntityId } }
  | {
      anchor: {
        document: { id: EntityId };
        ownerUid: string;
        anchorId: { value: string };
        expectedDigest?: { sha256: string };
      };
    };
export interface CatalogCheckedOwner {
  owner?: NativeEntityRef;
  uid?: string;
  etag?: string;
}
export interface CatalogInspection {
  entity?: Record<string, unknown>;
  etag?: string;
  anchor?: Record<string, unknown>;
  declarations?: Record<string, unknown>[];
  scope?: {
    branch?: string;
    revision?: string;
    authorizedNamespaces?: string[];
    visited?: CatalogCheckedOwner[];
    visitedSubjects?: CatalogSubject[];
    visitedDocuments?: DocumentSelection[];
  };
  incomplete?: boolean;
  failures?: { field?: string; connectionId?: { value: string }; code?: number; message?: string }[];
  selectionState?: string;
  subjectState?: string;
  replacementChain?: CatalogCheckedOwner[];
}

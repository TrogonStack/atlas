import type { WireEntityEnvelope } from '@/lib/api';
import type { EntityId } from '@/lib/model';

export interface GlossarySelection {
  namespaces: readonly string[];
  audience?: string;
  includeUnreviewed?: boolean;
}

export interface GlossaryTermRef {
  id?: EntityId;
}

export interface GlossaryDocumentRef {
  id?: EntityId;
}

export interface GlossaryIndexEntry {
  term?: GlossaryTermRef;
  ownerUid?: string;
  etag?: string;
  title?: string;
  audience?: string;
  state?: string;
  published?: boolean;
  reason?: string;
  nextReviewOn?: { value?: string };
}

export interface GlossaryIndex {
  entries?: GlossaryIndexEntry[];
  nextPageToken?: string;
  incomplete?: boolean;
  snapshotRevision?: string;
}

export interface GlossaryEvidenceResolution {
  evidenceIndex?: number;
  state?: string;
  reason?: string;
}

export interface GlossaryTermDetail {
  entity?: WireEntityEnvelope;
  etag?: string;
  entry?: GlossaryIndexEntry;
  evidence?: GlossaryEvidenceResolution[];
  incomplete?: boolean;
}

export interface GlossarySourceMaterial {
  document?: GlossaryDocumentRef;
  ownerUid?: string;
  etag?: string;
  contentDigest?: { sha256?: string };
  text?: string;
  state?: string;
}

export interface GlossarySources {
  sources?: GlossarySourceMaterial[];
  incomplete?: boolean;
  snapshotRevision?: string;
}

export interface GlossaryComparison {
  differences?: {
    kind?: string;
    left?: GlossaryTermRef;
    right?: GlossaryTermRef;
    title?: string;
    explanation?: string;
  }[];
  incomplete?: boolean;
  snapshotRevision?: string;
}

export interface GlossaryRefresh {
  observations?: {
    entry?: GlossaryIndexEntry;
    evidence?: GlossaryEvidenceResolution[];
    generationChanged?: boolean;
  }[];
  sources?: GlossarySourceMaterial[];
  incomplete?: boolean;
  snapshotRevision?: string;
}

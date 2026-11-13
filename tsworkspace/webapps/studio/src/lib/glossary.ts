import type { WireEntityEnvelope } from '@/lib/api';
import type { EntityId } from '@/lib/model';
import { withBranchQuery } from '@/lib/branch';
import { isSafeNamespace } from '../../shared/safe-namespace.mjs';

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

export type GlossaryEntryState =
  | 'GLOSSARY_ENTRY_STATE_UNSPECIFIED'
  | 'GLOSSARY_ENTRY_STATE_DRAFT'
  | 'GLOSSARY_ENTRY_STATE_REVIEWED'
  | 'GLOSSARY_ENTRY_STATE_REJECTED'
  | 'GLOSSARY_ENTRY_STATE_NEEDS_CHANGES'
  | 'GLOSSARY_ENTRY_STATE_STALE_REVIEW'
  | 'GLOSSARY_ENTRY_STATE_SOURCE_CHANGED'
  | 'GLOSSARY_ENTRY_STATE_SOURCE_UNAVAILABLE'
  | 'GLOSSARY_ENTRY_STATE_REVIEW_DUE';

export type GlossaryEvidenceState =
  | 'GLOSSARY_EVIDENCE_STATE_UNSPECIFIED'
  | 'GLOSSARY_EVIDENCE_STATE_CURRENT'
  | 'GLOSSARY_EVIDENCE_STATE_CHANGED'
  | 'GLOSSARY_EVIDENCE_STATE_UNAVAILABLE';

export interface GlossaryIndexEntry {
  term?: GlossaryTermRef;
  ownerUid?: string;
  etag?: string;
  title?: string;
  audience?: string;
  state?: GlossaryEntryState;
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
  state?: GlossaryEvidenceState;
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
  state?: GlossaryEvidenceState;
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

const STATE_LABELS: Record<string, string> = {
  GLOSSARY_ENTRY_STATE_DRAFT: 'Draft',
  GLOSSARY_ENTRY_STATE_REVIEWED: 'Reviewed',
  GLOSSARY_ENTRY_STATE_REJECTED: 'Rejected',
  GLOSSARY_ENTRY_STATE_NEEDS_CHANGES: 'Needs changes',
  GLOSSARY_ENTRY_STATE_STALE_REVIEW: 'Stale review',
  GLOSSARY_ENTRY_STATE_SOURCE_CHANGED: 'Source changed',
  GLOSSARY_ENTRY_STATE_SOURCE_UNAVAILABLE: 'Source unavailable',
  GLOSSARY_ENTRY_STATE_REVIEW_DUE: 'Review due',
};
export function glossaryStateLabel(state: string | undefined): string {
  return state ? (STATE_LABELS[state] ?? 'State unavailable') : 'State unavailable';
}
export function glossaryEvidenceLabel(state: string | undefined): string {
  if (!state) return 'Not checked';
  const labels: Record<string, string> = {
    GLOSSARY_EVIDENCE_STATE_CURRENT: 'Current',
    GLOSSARY_EVIDENCE_STATE_CHANGED: 'Changed',
    GLOSSARY_EVIDENCE_STATE_UNAVAILABLE: 'Unavailable',
  };
  return labels[state] ?? 'Source state unavailable';
}
export function glossaryRecord(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}
export function glossaryText(value: unknown): string {
  return typeof value === 'string' ? value : '';
}
export function exactGlossaryId(value: unknown): EntityId | undefined {
  const id = glossaryRecord(value);
  const version =
    typeof id.version === 'number' && Number.isSafeInteger(id.version) ? String(id.version) : glossaryText(id.version);
  if (
    typeof id.namespace !== 'string' ||
    typeof id.slug !== 'string' ||
    !isSafeNamespace(id.namespace) ||
    !isSafeNamespace(id.slug) ||
    !/^[1-9][0-9]*$/.test(version) ||
    version.length > 20 ||
    BigInt(version) > 18446744073709551615n
  )
    return undefined;
  return { namespace: id.namespace, slug: id.slug, version };
}
export function glossaryIdLabel(id: EntityId): string {
  return `${id.namespace}/${id.slug}@${id.version}`;
}
export function sameGlossaryId(a: EntityId | undefined, b: EntityId | undefined): boolean {
  return (
    a !== undefined && b !== undefined && a.namespace === b.namespace && a.slug === b.slug && a.version === b.version
  );
}
export function glossaryHref(id: EntityId, branch: string | undefined): string {
  return withBranchQuery(
    `/glossary?${new URLSearchParams({ namespace: id.namespace, slug: id.slug, version: id.version })}`,
    branch,
  );
}

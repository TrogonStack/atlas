// Tests for buildIssueIndex. Pure transformation over the wire shape of
// ValidateEventModelResponse.issues (service.proto: ValidationIssue).

import { describe, expect, it } from 'vitest';
import { buildIssueIndex, EMPTY_ISSUE_INDEX } from './issues';

// Shaped like GET /api/validate?namespace=registry&slug=changeset-review-flow.
const RM_EVENT_NOT_PROJECTED = {
  severity: 'SEVERITY_WARNING',
  code: 'RM_EVENT_NOT_PROJECTED',
  message:
    "read model registry/changesets-awaiting-review@1 declares event registry/changeset.rejected@1 as a source but no in-model read-model slice projects it; add a rmSlice that consumes it or drop the event from the read model's source_events",
  subject: {
    kind: 'ENTITY_KIND_READ_MODEL',
    id: { namespace: 'registry', slug: 'changesets-awaiting-review', version: '1' },
  },
  subjectField: 'source_events',
  ruleTitle: 'Read model declares an event nothing projects',
  ruleCategory: 'model',
  ruleDefaultSeverity: 'SEVERITY_WARNING',
};

const subject = {
  kind: 'readModel' as const,
  id: { namespace: 'registry', slug: 'changesets-awaiting-review', version: '1' },
};

describe('buildIssueIndex', () => {
  it('attaches a finding to the entity named by its subject', () => {
    const idx = buildIssueIndex([RM_EVENT_NOT_PROJECTED]);
    const found = idx.for(subject);
    expect(found).toHaveLength(1);
    expect(found[0].code).toBe('RM_EVENT_NOT_PROJECTED');
    expect(found[0].severity).toBe('warning');
    expect(found[0].field).toBe('source_events');
    expect(found[0].ruleTitle).toBe('Read model declares an event nothing projects');
  });

  it('returns no findings for an entity nothing was reported against', () => {
    const idx = buildIssueIndex([RM_EVENT_NOT_PROJECTED]);
    expect(
      idx.for({ kind: 'event', id: { namespace: 'registry', slug: 'changeset.submitted', version: '1' } }),
    ).toEqual([]);
  });

  it('does not leak findings across versions of the same slug', () => {
    const idx = buildIssueIndex([RM_EVENT_NOT_PROJECTED]);
    expect(idx.for({ ...subject, id: { ...subject.id, version: '2' } })).toEqual([]);
  });

  it('orders findings worst first so a truncated read leads with what breaks', () => {
    const idx = buildIssueIndex([
      { ...RM_EVENT_NOT_PROJECTED, severity: 'SEVERITY_INFO', code: 'SCHEMA_MISSING' },
      { ...RM_EVENT_NOT_PROJECTED, severity: 'SEVERITY_ERROR', code: 'DANGLING_REF' },
      RM_EVENT_NOT_PROJECTED,
    ]);
    expect(idx.for(subject).map((i) => i.severity)).toEqual(['error', 'warning', 'info']);
    expect(idx.counts).toEqual({ error: 1, warning: 1, info: 1 });
  });

  it('keeps a finding with no resolvable subject instead of dropping it', () => {
    const idx = buildIssueIndex([{ severity: 'SEVERITY_ERROR', code: 'CROSS_MODEL_BOUNDARY_VIOLATION', message: 'm' }]);
    expect(idx.unattached.map((i) => i.code)).toEqual(['CROSS_MODEL_BOUNDARY_VIOLATION']);
    expect(idx.counts.error).toBe(1);
  });

  it('drops a finding whose severity it cannot read rather than understating it', () => {
    const idx = buildIssueIndex([{ ...RM_EVENT_NOT_PROJECTED, severity: 'SEVERITY_UNSPECIFIED' }]);
    expect(idx.all).toEqual([]);
  });

  it('records the rule default when the reported severity was downgraded for this case', () => {
    const idx = buildIssueIndex([
      { ...RM_EVENT_NOT_PROJECTED, severity: 'SEVERITY_INFO', ruleDefaultSeverity: 'SEVERITY_ERROR' },
    ]);
    expect(idx.for(subject)[0]).toMatchObject({ severity: 'info', defaultSeverity: 'error' });
  });

  it('survives a non-array payload', () => {
    expect(buildIssueIndex(undefined).all).toEqual([]);
    expect(buildIssueIndex({ issues: [] }).all).toEqual([]);
  });

  it('carries a validation failure so "did not run" is distinguishable from "found nothing"', () => {
    const failed = buildIssueIndex(undefined, 'validate: 503');
    expect(failed.error).toBe('validate: 503');
    expect(failed.all).toEqual([]);
    expect(EMPTY_ISSUE_INDEX.error).toBeUndefined();
  });
});

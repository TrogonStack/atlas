// Validation findings from ValidateEventModel, indexed by the entity they
// attach to.
//
// on purpose: the server already decides what is wrong with a model, and
// until this existed nothing in the Studio ever asked. A read model could
// declare a source event no slice projects (RM_EVENT_NOT_PROJECTED) and the
// board, the inspector, and the copied context would all describe it as
// healthy. Rendering the same finding the validator emits is the whole job
// here: this module invents no rules and knows no rule codes.

import { type EntityId, type EntityKind, entityKey, wireKind } from './model';

export type Severity = 'error' | 'warning' | 'info';

const WIRE_SEVERITY: Record<string, Severity> = {
  SEVERITY_INFO: 'info',
  SEVERITY_WARNING: 'warning',
  SEVERITY_ERROR: 'error',
};

/** Worst first, so a truncated read still leads with what breaks. */
const RANK: Record<Severity, number> = { error: 0, warning: 1, info: 2 };

export interface Issue {
  severity: Severity;
  /** Stable machine code, e.g. `RM_EVENT_NOT_PROJECTED`. */
  code: string;
  message: string;
  /** Structural path inside the entity, e.g. `source_events`. */
  field: string;
  /** Short label from the rule catalog. Empty when the code is unknown to it. */
  ruleTitle: string;
  /** `model` | `scenarios` | `strategic` | `schema` | `storyboard`. */
  ruleCategory: string;
  /**
   * The rule's canonical severity. Some rules downgrade conditionally, so
   * this can be stricter than `severity`; surfacing both explains why a
   * finding titled like an error is reported as info.
   */
  defaultSeverity?: Severity;
  subject?: { kind: EntityKind; id: EntityId };
}

export interface IssueIndex {
  /** Every parsed finding, worst first. */
  all: Issue[];
  /** Findings whose subject is absent or unresolvable: model-wide, or a kind we cannot map. */
  unattached: Issue[];
  counts: Record<Severity, number>;
  /** Why validation is unavailable. Set means `all` is empty because nothing ran, not because nothing is wrong. */
  error?: string;
  /** Findings on one entity, worst first. */
  for(entity: { kind: EntityKind; id: EntityId }): Issue[];
}

const NONE: Issue[] = [];

function parseIssue(raw: unknown): Issue | undefined {
  if (!raw || typeof raw !== 'object') return undefined;
  const r = raw as Record<string, unknown>;
  // An unrecognized severity is dropped rather than guessed: reporting an
  // unknown finding as `info` would understate it.
  const severity = WIRE_SEVERITY[String(r.severity ?? '')];
  if (!severity) return undefined;
  const subjectRaw = r.subject as { kind?: unknown; id?: unknown } | undefined;
  const kind = wireKind(subjectRaw?.kind);
  const id = subjectRaw?.id as Partial<EntityId> | undefined;
  const subject =
    kind && id?.namespace && id?.slug
      ? { kind, id: { namespace: id.namespace, slug: id.slug, version: String(id.version ?? '1') } }
      : undefined;
  return {
    severity,
    code: String(r.code ?? ''),
    message: String(r.message ?? ''),
    field: String(r.subjectField ?? ''),
    ruleTitle: String(r.ruleTitle ?? ''),
    ruleCategory: String(r.ruleCategory ?? ''),
    defaultSeverity: WIRE_SEVERITY[String(r.ruleDefaultSeverity ?? '')],
    subject,
  };
}

export function buildIssueIndex(raw: unknown, error?: string): IssueIndex {
  const parsed = (Array.isArray(raw) ? raw : [])
    .map(parseIssue)
    .filter((i): i is Issue => Boolean(i))
    .sort((a, b) => RANK[a.severity] - RANK[b.severity] || a.code.localeCompare(b.code));

  const byKey = new Map<string, Issue[]>();
  const unattached: Issue[] = [];
  const counts: Record<Severity, number> = { error: 0, warning: 0, info: 0 };
  for (const issue of parsed) {
    counts[issue.severity] += 1;
    if (!issue.subject) {
      unattached.push(issue);
      continue;
    }
    const key = entityKey(issue.subject.kind, issue.subject.id);
    const bucket = byKey.get(key);
    if (bucket) bucket.push(issue);
    else byKey.set(key, [issue]);
  }

  return {
    all: parsed,
    unattached,
    counts,
    error,
    for: (entity) => byKey.get(entityKey(entity.kind, entity.id)) ?? NONE,
  };
}

/** Shared empty index: no findings, and no claim that validation ran. */
export const EMPTY_ISSUE_INDEX: IssueIndex = buildIssueIndex(undefined);

import { useEffect, useState } from 'react';
import {
  getTeamContacts,
  listAssigneeResponsibilities,
  listPersonTeams,
  listSubjectAssignments,
  listTeamMembers,
} from '@/lib/api';
import { readAsOfFromUrl, readBranchFromUrl } from '@/lib/branch';
import type { Entity } from '@/lib/model';
import {
  directoryAssignee,
  responsibilitySubjects,
  type ResponsibilityAssignment,
  type ResponsibilityAssignments,
  type ResponsibilityContacts,
  type ResponsibilityKind,
  type ResponsibilityMemberships,
  type ResponsibilityQueryContext,
} from '@/lib/responsibility';

function Exact({ value }: { value: unknown }) {
  return (
    <pre className="whitespace-pre-wrap break-words text-xs">{JSON.stringify(value, null, 2) ?? 'Undeclared'}</pre>
  );
}
function resolution(state?: string): string {
  return state?.replace('RESPONSIBILITY_RESOLUTION_STATE_', '').toLowerCase() || 'unchecked';
}
function Assignments({ data }: { data: ResponsibilityAssignments }) {
  return (
    <>
      {data.subjectState && <p>Backend subject state: {resolution(data.subjectState)}</p>}
      {(data.assignments ?? []).map((assignment: ResponsibilityAssignment) => (
        <section className="flex flex-col gap-1 rounded border p-2" key={JSON.stringify(assignment)}>
          <p>Backend assignment state: {resolution(assignment.state)}</p>
          <Exact value={assignment} />
        </section>
      ))}
      {!data.assignments?.length && (
        <p>
          {data.subjectState === 'RESPONSIBILITY_RESOLUTION_STATE_INACCESSIBLE'
            ? 'Subject inaccessible; direct assignments remain unknown.'
            : data.subjectState === 'RESPONSIBILITY_RESOLUTION_STATE_UNRESOLVED'
              ? 'Subject unresolved; the retained exact selector may be stale or removed.'
              : data.incomplete || !data.scope || data.subjectState === 'RESPONSIBILITY_RESOLUTION_STATE_UNSPECIFIED'
                ? 'No assignment records returned; complete visible absence remains unproven.'
                : 'No direct assignments returned in the checked authorized scope. This does not establish operational responsibility or responsibility for contained subjects.'}
        </p>
      )}
      {data.assignments?.some((item) => item.state === 'RESPONSIBILITY_RESOLUTION_STATE_UNRESOLVED') && (
        <p>Unresolved retained selectors may be stale or removed. No replacement was selected.</p>
      )}
    </>
  );
}
function BackendQuery<T extends ResponsibilityQueryContext>({
  title,
  load,
  children,
}: {
  title: string;
  load: (signal: AbortSignal) => Promise<T>;
  children: (data: T) => React.ReactNode;
}) {
  const [result, setResult] = useState<{ data?: T; error?: string }>();
  useEffect(() => {
    const controller = new AbortController();
    load(controller.signal).then(
      (data) => {
        if (!controller.signal.aborted) setResult({ data });
      },
      (error: unknown) => {
        if (!controller.signal.aborted) setResult({ error: error instanceof Error ? error.message : String(error) });
      },
    );
    return () => controller.abort();
  }, [load]);
  return (
    <section className="flex flex-col gap-2">
      <h4 className="font-semibold">{title}</h4>
      {!result && <p>Checking backend context…</p>}
      {result?.error && (
        <p role="alert">Backend inspection unavailable: {result.error}. Responsibility remains unknown.</p>
      )}
      {result?.data && (
        <>
          <p>Backend checked context</p>
          <Exact value={result.data.scope} />
          {!result.data.scope && <p>Checked scope unavailable; no complete conclusion follows.</p>}
          {result.data.incomplete && <p>Incomplete backend inspection; missing records do not prove absence.</p>}
          {children(result.data)}
        </>
      )}
    </section>
  );
}
function Memberships({ data }: { data: ResponsibilityMemberships }) {
  return (
    <>
      {(data.memberships ?? []).map((membership) => (
        <div key={JSON.stringify(membership)}>
          <p>Backend membership state: {resolution(membership.state)}</p>
          <Exact value={membership} />
        </div>
      ))}
      {!data.memberships?.length && (
        <p>No memberships returned in the checked visible scope; external-directory membership remains unknown.</p>
      )}
      <p>Membership confers no inherited responsibility.</p>
    </>
  );
}
function Contacts({ data, includeMembers }: { data: ResponsibilityContacts; includeMembers: boolean }) {
  return (
    <>
      <h5 className="font-semibold">Authored Team contacts</h5>
      <Exact value={data.teamContacts ?? []} />
      <h5 className="font-semibold">Expanded member contacts with membership provenance</h5>
      {includeMembers ? <Exact value={data.memberContacts ?? []} /> : <p>Member contact expansion not requested.</p>}
      <p>Contacts are catalog declarations. Member contacts are separate from direct Person assignments.</p>
    </>
  );
}

const kinds: ResponsibilityKind[] = [
  'RESPONSIBILITY_KIND_OWNER',
  'RESPONSIBILITY_KIND_MAINTAINER',
  'RESPONSIBILITY_KIND_STEWARD',
  'RESPONSIBILITY_KIND_CONTACT',
];

export function ResponsibilityInspector({ entity, asOf: historicalAsOf }: { entity: Entity; asOf?: string }) {
  const asOf = historicalAsOf ?? readAsOfFromUrl();
  const branch = asOf ? undefined : readBranchFromUrl();
  const subjects = responsibilitySubjects(entity);
  const [selection, setSelection] = useState<{ entityKey: string; subject: string }>();
  const [kind, setKind] = useState<ResponsibilityKind>();
  const [includeMembers, setIncludeMembers] = useState(false);
  const selected =
    selection?.entityKey === entity.key
      ? subjects.find((item) => JSON.stringify(item.subject) === selection.subject)
      : undefined;
  const subject = selected?.subject ?? subjects[0].subject;
  const assignee = directoryAssignee(entity);
  const directory = entity.kind === 'team' || entity.kind === 'person';
  const contextKey = JSON.stringify({
    entity: entity.key,
    raw: entity.raw,
    uid: entity.uid,
    branch,
    asOf,
    subject,
    kind,
    includeMembers,
  });
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">Responsibility inspection</h3>
      <p>Catalog assignments do not grant authorization or establish execution, lineage or exclusive ownership.</p>
      {directory && (
        <section className="flex flex-col gap-2">
          <h4 className="font-semibold">
            {asOf ? 'Retained historical profile and declarations' : 'Loaded profile and authored declarations'}
          </h4>
          {entity.kind === 'person' && (
            <p>Person is an unverified directory profile, separate from Persona and authenticated identity.</p>
          )}
          <Exact
            value={{
              id: entity.id,
              ownerUid: entity.uid,
              title: entity.raw.title,
              purpose: entity.raw.purpose,
              role: entity.raw.role,
              directoryPointer: entity.raw.directoryPointer,
              contacts: entity.raw.contacts,
              members: entity.raw.members,
              responsibilities: entity.raw.responsibilities,
            }}
          />
        </section>
      )}
      <label className="flex flex-col gap-1">
        Exact responsibility subject
        <select
          value={JSON.stringify(subject)}
          onChange={(event) => setSelection({ entityKey: entity.key, subject: event.target.value })}
        >
          {subjects.map((item) => (
            <option key={JSON.stringify(item)} value={JSON.stringify(item.subject)}>
              {item.label}
            </option>
          ))}
        </select>
      </label>
      <Exact value={subject} />
      {asOf ? (
        <>
          <p>
            Historical responsibility inspection unavailable; retained declarations only. Current directory hydration is
            not attached to this historical view.
          </p>
          <Exact value={{ asOf, entity: entity.key }} />
        </>
      ) : (
        <>
          <label className="flex flex-col gap-1">
            Responsibility kind
            <select
              value={kind ?? ''}
              onChange={(event) => setKind(event.target.value ? (event.target.value as ResponsibilityKind) : undefined)}
            >
              <option value="">All kinds</option>
              {kinds.map((item) => (
                <option key={item} value={item}>
                  {item.replace('RESPONSIBILITY_KIND_', '')}
                </option>
              ))}
            </select>
          </label>
          {entity.kind === 'team' && (
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={includeMembers}
                onChange={(event) => setIncludeMembers(event.target.checked)}
              />
              Include authorized member contacts
            </label>
          )}
          <div key={contextKey} className="flex flex-col gap-4">
            <BackendQuery
              title="Direct assignments for exact subject"
              load={(signal) => listSubjectAssignments({ subject, kind }, { branch, signal })}
            >
              {(data) => <Assignments data={data} />}
            </BackendQuery>
            {assignee && (
              <BackendQuery
                title="Authored assignee responsibilities"
                load={(signal) => listAssigneeResponsibilities({ assignee, kind }, { branch, signal })}
              >
                {(data) => <Assignments data={data} />}
              </BackendQuery>
            )}
            {entity.kind === 'team' && (
              <>
                <BackendQuery
                  title="Team membership"
                  load={(signal) => listTeamMembers({ id: entity.id }, { branch, signal })}
                >
                  {(data) => <Memberships data={data} />}
                </BackendQuery>
                <BackendQuery
                  title="Team contact lookup"
                  load={(signal) => getTeamContacts({ id: entity.id }, includeMembers, { branch, signal })}
                >
                  {(data) => <Contacts data={data} includeMembers={includeMembers} />}
                </BackendQuery>
              </>
            )}
            {entity.kind === 'person' && (
              <BackendQuery
                title="Person Teams"
                load={(signal) => listPersonTeams({ id: entity.id }, { branch, signal })}
              >
                {(data) => <Memberships data={data} />}
              </BackendQuery>
            )}
          </div>
        </>
      )}
    </section>
  );
}

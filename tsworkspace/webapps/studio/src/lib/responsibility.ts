import type { Entity, EntityId } from '@/lib/model';

export interface DirectoryValue {
  value: string;
}
export interface NativeEntityRef {
  kind: string;
  id: EntityId;
}
export interface DirectoryRef {
  id: EntityId;
}
export type ResponsibilityKind =
  | 'RESPONSIBILITY_KIND_OWNER'
  | 'RESPONSIBILITY_KIND_MAINTAINER'
  | 'RESPONSIBILITY_KIND_STEWARD'
  | 'RESPONSIBILITY_KIND_CONTACT';
export type ResponsibilitySubject =
  | { entity: NativeEntityRef }
  | { connection: { owner: NativeEntityRef; ownerUid: string; id: DirectoryValue } }
  | {
      member: {
        specification: DirectoryRef;
        ownerUid: string;
        kind: string;
        memberId: DirectoryValue;
        expectedDigest?: { sha256: string };
      };
    }
  | {
      port: {
        product: DirectoryRef;
        ownerUid: string;
        direction: 'DATA_PORT_DIRECTION_INPUT' | 'DATA_PORT_DIRECTION_OUTPUT';
        portId: DirectoryValue;
      };
    };
export type ResponsibilityAssignee = { team: DirectoryRef } | { person: DirectoryRef };
export interface ResponsibilityDeclaration {
  connectionId?: DirectoryValue;
  kind?: ResponsibilityKind;
  subject?: ResponsibilitySubject;
  doc?: string;
}
export interface TeamMemberDeclaration {
  connectionId?: DirectoryValue;
  person?: DirectoryRef;
  doc?: string;
}
export type ContactPoint = { label?: string } & (
  | { email: DirectoryValue }
  | { web: DirectoryValue }
  | { chat: { service: DirectoryValue; handle: DirectoryValue } }
);
export type ResponsibilityResolutionState =
  | 'RESPONSIBILITY_RESOLUTION_STATE_UNSPECIFIED'
  | 'RESPONSIBILITY_RESOLUTION_STATE_RESOLVED'
  | 'RESPONSIBILITY_RESOLUTION_STATE_UNRESOLVED'
  | 'RESPONSIBILITY_RESOLUTION_STATE_INACCESSIBLE';
export interface ResponsibilityCheckedScope {
  branch?: string;
  authorizedNamespaces?: string[];
  revision?: string;
  visited?: NativeEntityRef[];
  visitedSubjects?: ResponsibilitySubject[];
}
export interface ResponsibilityQueryContext {
  scope?: ResponsibilityCheckedScope;
  incomplete?: boolean;
}
export interface ResponsibilityAssignment {
  assignee?: ResponsibilityAssignee;
  assigneeUid?: string;
  assignment?: ResponsibilityDeclaration;
  state?: ResponsibilityResolutionState;
}
export interface ResponsibilityMembership {
  team?: DirectoryRef;
  teamUid?: string;
  membership?: TeamMemberDeclaration;
  state?: ResponsibilityResolutionState;
}
export interface ResponsibilityContact {
  team?: DirectoryRef;
  teamUid?: string;
  membershipId?: DirectoryValue;
  person?: DirectoryRef;
  contact?: ContactPoint;
  state?: ResponsibilityResolutionState;
}
export interface ResponsibilityAssignments extends ResponsibilityQueryContext {
  assignments?: ResponsibilityAssignment[];
  subjectState?: ResponsibilityResolutionState;
}
export interface ResponsibilityMemberships extends ResponsibilityQueryContext {
  memberships?: ResponsibilityMembership[];
}
export interface ResponsibilityContacts extends ResponsibilityQueryContext {
  teamContacts?: ResponsibilityContact[];
  memberContacts?: ResponsibilityContact[];
}
export interface SubjectAssignmentsRequest {
  subject: ResponsibilitySubject;
  kind?: ResponsibilityKind;
}
export interface AssigneeResponsibilitiesRequest {
  assignee: ResponsibilityAssignee;
  kind?: ResponsibilityKind;
}

export function nativeEntityRef(entity: Entity): NativeEntityRef {
  return {
    kind: `ENTITY_KIND_${entity.kind.replace(/[A-Z]/g, (letter) => `_${letter}`).toUpperCase()}`,
    id: entity.id,
  };
}

export function directoryAssignee(entity: Entity): ResponsibilityAssignee | undefined {
  if (entity.kind === 'team') return { team: { id: entity.id } };
  if (entity.kind === 'person') return { person: { id: entity.id } };
  return undefined;
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}
const occurrencePaths: Partial<Record<Entity['kind'], string[]>> = {
  commandSlice: ['persona', 'ui', 'command', 'emittedEvents.*'],
  readModelSlice: ['sourceEvents.*', 'readModel'],
  automationSlice: ['sourceReadModels.*', 'processor', 'emittedCommand'],
  uiSlice: ['persona', 'sourceReadModels.*', 'ui'],
  storyboard: [
    'entry.readModel',
    'entry.human.persona',
    'entry.human.ui',
    'entry.automation.processor',
    'outcome.events.*',
    'outcome.ui',
  ],
  swimlane: ['transitions.*.next.*'],
  ui: ['transitions.*'],
  component: ['relationships.*'],
  externalSystem: ['relationships.*'],
  channel: ['bindings.*'],
  dataStore: ['attachments.*'],
  specification: [
    'schemas.*.definition',
    'messages.*.schemas.*',
    'messages.*.mapping',
    'operations.*.messages.*',
    'operations.*.binding.channel',
    'exposures.*',
    'usages.*',
    'dependencies.*',
  ],
  dataProduct: [
    'inputs.*.data',
    'outputs.*.data',
    'outputs.*.contract',
    'outputs.*.access.*',
    'outputs.*.derivedFrom.inputs.*',
  ],
};

export function responsibilitySubjects(entity: Entity): { label: string; subject: ResponsibilitySubject }[] {
  const owner = nativeEntityRef(entity);
  const subjects: { label: string; subject: ResponsibilitySubject }[] = [
    { label: 'Exact entity', subject: { entity: owner } },
  ];
  const authored = entity.raw.responsibilities;
  if ((entity.kind === 'team' || entity.kind === 'person') && Array.isArray(authored)) {
    authored.forEach((assignment) => {
      const body = record(assignment);
      if (body.subject)
        subjects.push({
          label: `Authored assignment ${record(body.connectionId).value ?? 'without ID'}`,
          subject: body.subject as ResponsibilitySubject,
        });
    });
  }
  const ownerUid = entity.uid;
  if (!ownerUid) return subjects;
  const visit = (
    value: unknown,
    parts: string[],
    path: string,
    found: (value: Record<string, unknown>, path: string) => void,
  ) => {
    if (!parts.length) return found(record(value), path);
    const [part, ...rest] = parts;
    if (part === '*') {
      if (Array.isArray(value)) value.forEach((item, index) => visit(item, rest, `${path}[${index}]`, found));
    } else visit(record(value)[part], rest, path ? `${path}.${part}` : part, found);
  };
  for (const path of occurrencePaths[entity.kind] ?? []) {
    visit(entity.raw, path.split('.'), '', (value, label) => {
      const id = record(value.connectionId).value;
      if (typeof id === 'string' && id)
        subjects.push({ label: `${label}: ${id}`, subject: { connection: { owner, ownerUid, id: { value: id } } } });
    });
  }
  if (entity.kind === 'dataProduct') {
    for (const [field, direction] of [
      ['inputs', 'DATA_PORT_DIRECTION_INPUT'],
      ['outputs', 'DATA_PORT_DIRECTION_OUTPUT'],
    ] as const) {
      visit(entity.raw, [field, '*'], '', (value, label) => {
        const portId = record(value.portId).value;
        if (typeof portId === 'string' && portId)
          subjects.push({
            label: `${label}: ${portId}`,
            subject: { port: { product: { id: entity.id }, ownerUid, direction, portId: { value: portId } } },
          });
      });
    }
  }
  if (entity.kind === 'specification') {
    for (const [field, kind] of [
      ['artifacts', 'ARTIFACT'],
      ['schemas', 'SCHEMA'],
      ['messages', 'MESSAGE'],
      ['operations', 'OPERATION'],
    ]) {
      visit(entity.raw, [field, '*'], '', (value, label) => {
        const memberId = record(value.memberId).value;
        if (typeof memberId === 'string' && memberId)
          subjects.push({
            label: `${label}: ${memberId}`,
            subject: {
              member: {
                specification: { id: entity.id },
                ownerUid,
                kind: `CONTRACT_MEMBER_KIND_${kind}`,
                memberId: { value: memberId },
              },
            },
          });
      });
    }
  }
  return subjects;
}

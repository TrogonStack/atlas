import path from 'node:path';
import protoLoader from '@grpc/proto-loader';
import protobuf from 'protobufjs';
import { validateResponsibilityRequest } from './responsibility-request.mjs';
import { describe, expect, it } from 'vitest';

const protoRoot = path.resolve(import.meta.dirname, '../../../../proto');
const definitions = protoLoader.loadSync(path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'), {
  keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [protoRoot],
});
const id = { namespace: 'catalog', slug: 'service', version: '9007199254740993' };
const ref = { id };
const subjects = [
  { entity: { kind: 'ENTITY_KIND_DATA_PRODUCT', id } },
  { connection: { owner: { kind: 'ENTITY_KIND_AUTOMATION_SLICE', id }, ownerUid: 'edge-owner', id: { value: 'edge-b' } } },
  { member: { specification: ref, ownerUid: 'spec-owner', kind: 'CONTRACT_MEMBER_KIND_ARTIFACT', memberId: { value: 'artifact-a' }, expectedDigest: { sha256: 'a'.repeat(64) } } },
  { port: { product: ref, ownerUid: 'product-owner', direction: 'DATA_PORT_DIRECTION_OUTPUT', portId: { value: 'output-b' } } },
];
function roundTrip(name, value) {
  const codec = definitions[`trogonatlas.api.eventmodel.v1alpha1.${name}`];
  return codec.deserialize(codec.serialize(value));
}
describe('responsibility production wire contract', () => {
  it('exposes Team/Person envelopes and the native read RPCs', () => {
    const codec = definitions['trogonatlas.eventmodel.v1alpha1.Entity'];
    for (const kind of ['team', 'person']) {
      const entity = { [kind]: { id, title: { value: 'Directory profile' }, responsibilities: [{ connectionId: { value: 'assignment-a' }, kind: 'RESPONSIBILITY_KIND_STEWARD', subject: subjects[3] }] } };
      expect(codec.deserialize(codec.serialize(entity))).toEqual(entity);
    }
    const service = definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService'];
    for (const rpc of ['ListSubjectAssignments', 'ListAssigneeResponsibilities', 'ListTeamMembers', 'ListPersonTeams', 'GetTeamContacts']) {
      expect(Object.values(service).some((method) => method.path.endsWith(`/${rpc}`))).toBe(true);
    }
  });
  it('retains every exact subject, optional digest and filter across the real DTO codec', () => {
    const requests = [
      ...subjects.map((subject) => ['ListSubjectAssignmentsRequest', { subject, kind: 'RESPONSIBILITY_KIND_CONTACT' }]),
      ['ListAssigneeResponsibilitiesRequest', { assignee: { person: ref }, kind: 'RESPONSIBILITY_KIND_OWNER' }],
      ['ListTeamMembersRequest', { team: ref }],
      ['ListPersonTeamsRequest', { person: ref }],
      ['GetTeamContactsRequest', { team: ref, includeMembers: false }],
      ['GetTeamContactsRequest', { team: ref, includeMembers: true }],
    ];
    for (const [name, input] of requests) expect(roundTrip(name, input)).toEqual(input);
  });
  it('retains backend checked scope, assignment state, membership and distinct contact provenance', () => {
    const scope = { branch: 'review', revision: 'checked-revision', authorizedNamespaces: ['catalog'], visited: [{ kind: 'ENTITY_KIND_TEAM', id }], visitedSubjects: subjects };
    const assignments = [{ assignee: { team: ref }, assigneeUid: 'team-uid', assignment: { connectionId: { value: 'assignment-b' }, kind: 'RESPONSIBILITY_KIND_STEWARD', subject: subjects[2] }, state: 'RESPONSIBILITY_RESOLUTION_STATE_UNRESOLVED' }];
    const memberships = [{ team: ref, teamUid: 'team-uid', membership: { connectionId: { value: 'membership-a' }, person: ref }, state: 'RESPONSIBILITY_RESOLUTION_STATE_INACCESSIBLE' }];
    const contact = { team: ref, teamUid: 'team-uid', contact: { email: { value: 'team@example.test' } }, state: 'RESPONSIBILITY_RESOLUTION_STATE_RESOLVED' };
    const responses = [
      ['ListSubjectAssignmentsResponse', { scope, incomplete: true, assignments, subjectState: 'RESPONSIBILITY_RESOLUTION_STATE_INACCESSIBLE' }],
      ['ListAssigneeResponsibilitiesResponse', { scope, incomplete: false, assignments }],
      ['ListTeamMembersResponse', { scope, memberships, incomplete: true }],
      ['ListPersonTeamsResponse', { scope, memberships, incomplete: true }],
      ['GetTeamContactsResponse', { scope, incomplete: true, teamContacts: [contact], memberContacts: [{ ...contact, membershipId: { value: 'membership-a' }, person: ref }] }],
    ];
    for (const [name, output] of responses) expect(roundTrip(name, output)).toEqual(output);
  });
});


describe('strict responsibility requests before production encoding', () => {
  const root = new protobuf.Root();
  root.resolvePath = (_origin, target) => path.resolve(protoRoot, target);
  root.loadSync(path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'));
  it.each([
    ['ListSubjectAssignments', { subject: subjects[0], kind: 'RESPONSIBILITY_KIND_OWNER' }, { subject: subjects[0], kind: 'RESPONSIBILITY_KIND_OWNER' }],
    ['ListSubjectAssignments', { subject: { entity: { kind: 1, id } }, kind: 4 }, { subject: { entity: { kind: 'ENTITY_KIND_EVENT', id } }, kind: 'RESPONSIBILITY_KIND_CONTACT' }],
    ['ListSubjectAssignments', { subject: subjects[0], kind: 0 }, { subject: subjects[0], kind: 'RESPONSIBILITY_KIND_UNSPECIFIED' }],
    ['ListSubjectAssignments', { subject: { port: { ...subjects[3].port, direction: 2 } } }, { subject: subjects[3] }],
    ['ListSubjectAssignments', { subject: { member: { ...subjects[2].member, kind: 1 } } }, { subject: subjects[2] }],
    ['ListAssigneeResponsibilities', { assignee: { person: ref }, kind: 2 }, { assignee: { person: ref }, kind: 'RESPONSIBILITY_KIND_MAINTAINER' }],
    ['ListTeamMembers', { team: { id: { ...id, version: '18446744073709551615' } } }, { team: { id: { ...id, version: '18446744073709551615' } } }],
    ['ListPersonTeams', { person: { id: { ...id, version: 1 } } }, { person: { id: { ...id, version: '1' } } }],
    ['GetTeamContacts', { team: ref, includeMembers: false }, { team: ref, includeMembers: false }],
    ['GetTeamContacts', { team: ref }, { team: ref }],
  ])('preserves legitimate %s values and authored presence', (rpc, input, expected) => {
    const original = structuredClone(input);
    const type = root.lookupType(`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`);
    validateResponsibilityRequest(type, input);
    const service = definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService'];
    const method = Object.values(service).find((value) => value.path.endsWith(`/${rpc}`));
    const codec = definitions[`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`];
    expect(codec.deserialize(method.requestSerialize(input))).toEqual(expected);
    expect(input).toEqual(original);
  });
  it.each([
    ['ListSubjectAssignments', { subject: subjects[0], kind: 'TYPO' }],
    ['GetTeamContacts', { team: ref, includeMembers: 'false' }],
    ['ListSubjectAssignments', { subject: { entity: subjects[0].entity, port: subjects[3].port } }],
    ['ListSubjectAssignments', { subject: {} }],
    ['ListAssigneeResponsibilities', { assignee: {} }],
    ['ListTeamMembers', { team: { id: { ...id, version: {} } } }],
    ['ListSubjectAssignments', { subject: { connection: { owner: subjects[0].entity, ownerUid: 'uid', id: { value: 2 } } } }],
    ['ListSubjectAssignments', { subject: { member: { ...subjects[2].member, expected_digest: { sha256: 'a'.repeat(64) } } } }],
  ])('refuses malformed %s before it can lose presence or change meaning', (rpc, input) => {
    const type = root.lookupType(`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`);
    expect(() => validateResponsibilityRequest(type, input)).toThrow(/expected|unknown/);
  });
});

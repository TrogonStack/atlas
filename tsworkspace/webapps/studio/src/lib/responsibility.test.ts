import { describe, expect, it } from 'vitest';
import { buildModel, wireKind } from './model';
import { directoryAssignee, responsibilitySubjects } from './responsibility';

const id = { namespace: 'catalog', slug: 'service', version: '7' };
function entity(kind: string, raw: Record<string, unknown>, uid = 'retained-owner') {
  return buildModel([{ [kind]: { id, ...raw }, system: { uid } }]).entities[0];
}
describe('responsibility native inventory and selectors', () => {
  it('retains Team and Person separately from Persona, unwrapping display and role values', () => {
    const team = entity('team', { title: { value: 'Service team' } });
    const person = entity('person', { title: { value: 'Directory person' }, role: { value: 'Designer' } });
    expect(team.title).toBe('Service team');
    expect(person.title).toBe('Directory person');
    expect(person.role).toBe('Designer');
    expect(wireKind('ENTITY_KIND_TEAM')).toBe('team');
    expect(wireKind('ENTITY_KIND_PERSON')).toBe('person');
    expect(directoryAssignee(person)).toEqual({ person: { id } });
    expect(directoryAssignee(entity('persona', { title: 'Customer' }))).toBeUndefined();
  });
  it('selects exact repeated occurrences without endpoint or positional matching', () => {
    const model = entity('automationSlice', {
      sourceReadModels: ['edge-a', 'edge-b'].map((value) => ({ connectionId: { value }, id })),
    });
    const subjects = responsibilitySubjects(model).map((item) => item.subject);
    expect(subjects.slice(1)).toEqual(
      ['edge-a', 'edge-b'].map((value) => ({
        connection: { owner: { kind: 'ENTITY_KIND_AUTOMATION_SLICE', id }, ownerUid: 'retained-owner', id: { value } },
      })),
    );
    expect(responsibilitySubjects(entity('automationSlice', model.raw, ''))).toHaveLength(1);
    expect(
      responsibilitySubjects(entity('processor', { calls: [{ id, connectionId: { value: 'excluded' } }] })),
    ).toHaveLength(1);
  });
  it('retains ports, directions, member kinds and authored digest pin presence', () => {
    const ports = responsibilitySubjects(
      entity('dataProduct', { inputs: [{ portId: { value: 'in' } }], outputs: [{ portId: { value: 'out' } }] }),
    );
    expect(ports.slice(1).map((item) => item.subject)).toEqual(
      ['INPUT', 'OUTPUT'].map((direction, index) => ({
        port: {
          product: { id },
          ownerUid: 'retained-owner',
          direction: `DATA_PORT_DIRECTION_${direction}`,
          portId: { value: index ? 'out' : 'in' },
        },
      })),
    );
    const spec = responsibilitySubjects(
      entity(
        'specification',
        Object.fromEntries(
          ['artifacts', 'schemas', 'messages', 'operations'].map((field) => [field, [{ memberId: { value: field } }]]),
        ),
      ),
    );
    expect(spec).toHaveLength(5);
    const subject = {
      member: {
        specification: { id },
        ownerUid: 'old-owner',
        kind: 'CONTRACT_MEMBER_KIND_ARTIFACT',
        memberId: { value: 'artifact' },
        expectedDigest: { sha256: 'a'.repeat(64) },
      },
    };
    const authored = responsibilitySubjects(
      entity('team', {
        members: [{ connectionId: { value: 'membership' }, person: { id } }],
        responsibilities: [{ connectionId: { value: 'assignment' }, subject }],
      }),
    );
    expect(authored.map((item) => item.subject)).toEqual([{ entity: { kind: 'ENTITY_KIND_TEAM', id } }, subject]);
  });
});

import { expect, it } from 'vitest';
import { buildModel } from './model';
import { sliceOccurrences } from './human-interaction';
import { hostingConnection, resolveUiHostingSubject, uiHostings } from './ui-hosting';

const id = { namespace: 'shop', slug: 'portal', version: '1' };
function fixture() {
  return buildModel([
    {
      component: {
        id,
        title: 'Portal',
        uiHostings: [
          { connectionId: { value: 'hosting-a' }, ui: { id: { ...id, slug: 'review' } }, label: { value: 'Review' } },
        ],
      },
      system: { uid: 'owner-uid' },
    },
  ]);
}
it('resolves only a complete exact hosting selector without filling a missing version', () => {
  const model = fixture();
  const connection = {
    owner: { kind: 'ENTITY_KIND_COMPONENT', id },
    ownerUid: 'owner-uid',
    id: { value: 'hosting-a' },
  };
  expect(resolveUiHostingSubject(model, { connection })?.connectionId).toBe('hosting-a');
  for (const broken of [
    { ...connection, owner: { ...connection.owner, id: { namespace: 'shop', slug: 'portal' } } },
    { ...connection, owner: { ...connection.owner, kind: 'ENTITY_KIND_UI' } },
    { ...connection, ownerUid: '' },
    { ...connection, ownerUid: 'old-owner-uid' },
    { ...connection, id: {} },
  ])
    expect(resolveUiHostingSubject(model, { connection: broken })).toBeUndefined();
});
it('does not render an incomplete target as an exact UI', () => {
  const owner = fixture().entities[0];
  owner.raw.uiHostings = [
    {
      connectionId: { value: 'hosting-a' },
      ui: { id: { namespace: 'shop', slug: 'review' } },
      label: { value: 'Review' },
    },
  ];
  expect(uiHostings(owner)[0].ui).toBeUndefined();
});
it('does not construct a hosting connection from an incomplete containing owner', () => {
  const owner = fixture().entities[0];
  owner.raw.id = { namespace: 'shop', slug: 'portal' };
  expect(hostingConnection(uiHostings(owner)[0])).toBeUndefined();
});

it('constructs slice occurrences only from complete exact raw owner identities', () => {
  const sliceId = { namespace: 'shop', slug: 'issue', version: '18446744073709551615' };
  const model = buildModel([
    {
      commandSlice: {
        id: sliceId,
        title: 'Approve',
        persona: { connectionId: { value: 'persona-a' }, persona: { id } },
        ui: { connectionId: { value: 'ui-a' }, ui: { id } },
        command: { connectionId: { value: 'command-a' }, command: { id } },
      },
      system: { uid: 'slice-uid' },
    },
  ]);
  const owner = model.entities[0];
  expect(sliceOccurrences(owner, 'command')[0]?.connection).toEqual({
    owner: { kind: 'ENTITY_KIND_COMMAND_SLICE', id: sliceId },
    ownerUid: 'slice-uid',
    id: { value: 'command-a' },
  });
  for (const invalidId of [
    { namespace: 'shop', slug: 'issue' },
    { ...sliceId, version: '0' },
    { ...sliceId, version: '-1' },
    { ...sliceId, version: 9007199254740992 },
    { ...sliceId, version: '18446744073709551616' },
    { ...sliceId, namespace: '' },
    { ...sliceId, slug: '' },
    { ...sliceId, slug: 'different-key' },
  ]) {
    owner.raw.id = invalidId;
    expect(sliceOccurrences(owner, 'persona')).toEqual([]);
    expect(sliceOccurrences(owner, 'ui')).toEqual([]);
    expect(sliceOccurrences(owner, 'command')).toEqual([]);
  }
});

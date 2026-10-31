import { expect, it } from 'vitest';
import { buildModel } from './model';
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

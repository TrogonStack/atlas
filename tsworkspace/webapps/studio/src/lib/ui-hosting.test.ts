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

// Freshness: selectors and connections are derived from the current loaded model on every call.
// A retained selector from an earlier model state never resolves against a changed state.
const retained = {
  owner: { kind: 'ENTITY_KIND_COMPONENT', id },
  ownerUid: 'owner-uid',
  id: { value: 'hosting-a' },
};
it('stops resolving a retained hosting selector after the owner is recreated with a new uid', () => {
  const before = fixture();
  expect(resolveUiHostingSubject(before, { connection: retained })).toBeDefined();
  const recreated = buildModel([{ component: before.entities[0].raw, system: { uid: 'owner-uid-recreated' } }]);
  expect(resolveUiHostingSubject(recreated, { connection: retained })).toBeUndefined();
  expect(
    resolveUiHostingSubject(recreated, { connection: { ...retained, ownerUid: 'owner-uid-recreated' } }),
  ).toBeDefined();
});
it('stops resolving a retained hosting selector after the occurrence is removed or the owner version changes', () => {
  const removed = fixture();
  removed.entities[0].raw.uiHostings = [];
  expect(resolveUiHostingSubject(removed, { connection: retained })).toBeUndefined();
  const versioned = buildModel([
    { component: { ...fixture().entities[0].raw, id: { ...id, version: '2' } }, system: { uid: 'owner-uid' } },
  ]);
  expect(resolveUiHostingSubject(versioned, { connection: retained })).toBeUndefined();
  expect(resolveUiHostingSubject(buildModel([]), { connection: retained })).toBeUndefined();
});
it('resolves a retained hosting selector to the current wording and target, never the earlier ones', () => {
  const model = fixture();
  const first = resolveUiHostingSubject(model, { connection: retained });
  expect(first?.label).toBe('Review');
  model.entities[0].raw.uiHostings = [
    {
      connectionId: { value: 'hosting-a' },
      ui: { id: { ...id, slug: 'checkout' } },
      label: { value: 'Checkout' },
      doc: { value: 'new wording' },
    },
  ];
  const current = resolveUiHostingSubject(model, { connection: retained });
  expect(current?.label).toBe('Checkout');
  expect(current?.doc).toBe('new wording');
  expect(current?.ui?.id.slug).toBe('checkout');
  expect(first?.label).toBe('Review');
});
it('builds the hosting connection from the current owner uid and refuses a cleared uid', () => {
  const owner = fixture().entities[0];
  expect(hostingConnection(uiHostings(owner)[0])?.ownerUid).toBe('owner-uid');
  owner.uid = 'owner-uid-recreated';
  expect(hostingConnection(uiHostings(owner)[0])?.ownerUid).toBe('owner-uid-recreated');
  owner.uid = undefined;
  expect(hostingConnection(uiHostings(owner)[0])).toBeUndefined();
});
it('does not build a hosting connection for an occurrence whose identity was removed', () => {
  const owner = fixture().entities[0];
  const item = uiHostings(owner)[0];
  owner.raw.uiHostings = [{ ui: { id: { ...id, slug: 'review' } }, label: { value: 'Review' } }];
  expect(hostingConnection(uiHostings(owner)[0])).toBeUndefined();
  expect(item.connectionId).toBe('hosting-a');
});
it('derives slice occurrences from the current uid and occurrence list on every call', () => {
  const sliceId = { ...id, slug: 'issue' };
  const owner = buildModel([
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
  ]).entities[0];
  expect(sliceOccurrences(owner, 'ui')[0].connection.ownerUid).toBe('slice-uid');
  owner.uid = 'slice-uid-recreated';
  expect(sliceOccurrences(owner, 'ui')[0].connection.ownerUid).toBe('slice-uid-recreated');
  owner.raw.ui = { connectionId: { value: 'ui-renewed' }, ui: { id } };
  expect(sliceOccurrences(owner, 'ui').map((item) => item.connection.id.value)).toEqual(['ui-renewed']);
  owner.raw.ui = undefined;
  expect(sliceOccurrences(owner, 'ui')).toEqual([]);
  owner.uid = undefined;
  expect(sliceOccurrences(owner, 'command')).toEqual([]);
});

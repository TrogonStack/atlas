import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import protobuf from 'protobufjs';
import { afterEach, expect, it, vi } from 'vitest';
import { buildModel, entityKey, type Entity } from '@/lib/model';
import {
  channelLinks,
  incomingChannelLinks,
  incomingStorageClaims,
  resolveAssociationSubject,
  storageClaims,
} from '@/lib/channel-storage';
import { nativeEntityRef, responsibilitySubjects, type ResponsibilitySubject } from '@/lib/responsibility';
import { resolveArchitecturalSubject } from '@/lib/architectural-relationships';
import { resolveCommunicationSubject } from '@/lib/communication-intent';
import { ChannelStorageDetails } from './ChannelStorageDetails';
import { Inspector } from './Inspector';

vi.mock('@/lib/useEntityHomes', () => ({
  useEntityHomes: () => ({ homes: new Map(), error: undefined, retry: vi.fn() }),
}));
vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: () => <button type="button">Copy context</button>,
}));

const id = (slug: string, version = '18446744073709551615') => ({ namespace: 'shop', slug, version });
const link = (value: string, kind = 'CHANNEL_LINK_KIND_COMPOSES') => ({
  connectionId: { value },
  kind,
  target: { id: id('inner') },
  parameterValues: { tenant: 'acme' },
  doc: { value: '<script>inert()</script>\nwhy' },
});
const claim = (value: string, direction = 'STORAGE_DIRECTION_WRITE', extra: Record<string, unknown> = {}) => ({
  connectionId: { value },
  store: { id: id('warehouse') },
  direction,
  ...extra,
});
function fixture() {
  return buildModel([
    {
      channel: {
        id: id('hub'),
        title: 'Hub',
        links: [link('link-a'), link('link-b'), link('link-c', 'CHANNEL_LINK_KIND_ROUTES_TO')],
      },
      system: { uid: 'hub-uid' },
    },
    { channel: { id: id('inner'), title: 'Inner' } },
    {
      dataStore: {
        id: id('warehouse'),
        title: 'Warehouse',
        attachments: [{ connectionId: { value: 'attachment-a' }, readModel: { id: id('result') } }],
      },
      system: { uid: 'store-uid' },
    },
    {
      component: {
        id: id('checkout'),
        title: 'Checkout',
        storageClaims: [
          claim('claim-a', 'STORAGE_DIRECTION_READ', {
            attachment: {
              owner: { kind: 'ENTITY_KIND_DATA_STORE', id: id('warehouse') },
              ownerUid: 'store-uid',
              id: { value: 'attachment-a' },
            },
          }),
          claim('claim-b'),
          claim('claim-c'),
        ],
      },
      system: { uid: 'owner-uid' },
    },
    {
      dataProduct: {
        id: id('offering'),
        title: 'Offering',
        storageClaims: [claim('product-claim', 'STORAGE_DIRECTION_READ', { portId: { value: 'port-a' } })],
      },
    },
  ]);
}
afterEach(cleanup);

function selection(owner: Entity, value: string): ResponsibilitySubject {
  return { connection: { owner: nativeEntityRef(owner), ownerUid: owner.uid ?? '', id: { value } } };
}

it.each([
  ['binding collision', { connectionId: { value: 'link-a' } }],
  ['missing binding identity', {}],
])('rejects a Channel link subject with %s elsewhere in its owner', (_name, binding) => {
  const model = fixture();
  const owner = model.entities[0];
  const subject = selection(owner, 'link-a');
  expect(resolveAssociationSubject(model, subject)?.connectionId).toBe('link-a');
  owner.raw.bindings = [binding];
  expect(resolveAssociationSubject(model, subject)).toBeUndefined();
});

it.each([
  ['input selection', { inputs: [{ portId: { value: 'input' }, data: { connectionId: { value: 'product-claim' } } }] }],
  [
    'output selection',
    { outputs: [{ portId: { value: 'output' }, data: { connectionId: { value: 'product-claim' } } }] },
  ],
  [
    'output contract',
    { outputs: [{ portId: { value: 'output' }, contract: { connectionId: { value: 'product-claim' } } }] },
  ],
  [
    'output access',
    { outputs: [{ portId: { value: 'output' }, access: [{ connectionId: { value: 'product-claim' } }] }] },
  ],
  [
    'output lineage',
    {
      outputs: [
        { portId: { value: 'output' }, derivedFrom: { inputs: [{ connectionId: { value: 'product-claim' } }] } },
      ],
    },
  ],
  ['input port', { inputs: [{ portId: { value: 'product-claim' } }] }],
  ['output port', { outputs: [{ portId: { value: 'product-claim' } }] }],
  ['missing selection identity', { inputs: [{ portId: { value: 'input' }, data: {} }] }],
])('rejects a storage subject with a conflicting or incomplete %s', (_name, raw) => {
  const model = fixture();
  const owner = model.entities[4];
  owner.uid = 'product-uid';
  const subject = selection(owner, 'product-claim');
  expect(resolveAssociationSubject(model, subject)?.connectionId).toBe('product-claim');
  Object.assign(owner.raw, raw);
  expect(resolveAssociationSubject(model, subject)).toBeUndefined();
});

it('uses the shared storage inventory when resolving architecture and communication subjects', () => {
  const model = fixture();
  const owner = model.entities[3];
  owner.raw.relationships = [
    { connectionId: { value: 'relationship' }, target: { externalSystem: { id: id('peer') } } },
  ];
  owner.raw.communications = [{ connectionId: { value: 'communication' }, direction: 'COMMUNICATION_DIRECTION_SEND' }];
  expect(resolveArchitecturalSubject(model, selection(owner, 'relationship'))?.connectionId).toBe('relationship');
  expect(resolveCommunicationSubject(model, selection(owner, 'communication'))?.connectionId).toBe('communication');
  owner.raw.storageClaims = [claim('relationship'), claim('communication')];
  expect(resolveArchitecturalSubject(model, selection(owner, 'relationship'))).toBeUndefined();
  expect(resolveCommunicationSubject(model, selection(owner, 'communication'))).toBeUndefined();
});

it('includes ResourceGroups occurrence identities without counting target selectors as owned IDs', () => {
  const model = fixture();
  const owner = model.entities[3];
  const subject = selection(owner, 'claim-a');
  expect(resolveAssociationSubject(model, subject)?.connectionId).toBe('claim-a');
  owner.raw.metadata = [
    {
      '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation',
      groups: [{ groupId: { value: 'claim-a' }, items: [] }],
    },
  ];
  expect(resolveAssociationSubject(model, subject)).toBeUndefined();
});

it('keeps exact link kinds, equal siblings, parameter values and inert docs without inferring behavior', () => {
  const model = fixture();
  const hub = model.entities[0];
  expect(channelLinks(hub).map((item) => [item.path, item.connectionId, item.kind])).toEqual([
    ['links[0]', 'link-a', 'COMPOSES'],
    ['links[1]', 'link-b', 'COMPOSES'],
    ['links[2]', 'link-c', 'ROUTES_TO'],
  ]);
  expect(channelLinks(hub)[0].target?.id.version).toBe('18446744073709551615');
  expect(incomingChannelLinks(model, model.entities[1])).toHaveLength(3);
  const select = vi.fn();
  const { container } = render(<ChannelStorageDetails entity={hub} model={model} onSelect={select} />);
  expect(container.querySelector('script')).toBeNull();
  expect(screen.getAllByText('COMPOSES declared by the owner')).toHaveLength(2);
  expect(screen.getAllByText('tenant=acme')).toHaveLength(3);
  fireEvent.click(screen.getAllByRole('button', { name: 'shop/inner@18446744073709551615' })[0]);
  expect(select).toHaveBeenCalledWith(entityKey('channel', id('inner')));
  expect(screen.getByText(/do not establish routing, delivery or runtime behavior/)).toBeDefined();
  expect(model.slices).toHaveLength(0);
});

it('keeps READ and WRITE claims distinct, with exact attachment and DataProduct port, and loaded-only incoming rows', () => {
  const model = fixture();
  const component = model.entities[3];
  const claims = storageClaims(component);
  expect(claims.map((item) => [item.connectionId, item.direction])).toEqual([
    ['claim-a', 'READ'],
    ['claim-b', 'WRITE'],
    ['claim-c', 'WRITE'],
  ]);
  expect(claims[0].attachment?.ownerUid).toBe('store-uid');
  expect(claims[0].attachment?.id).toBe('attachment-a');
  expect(storageClaims(model.entities[4])[0].portId).toBe('port-a');
  expect(incomingStorageClaims(model, model.entities[2])).toHaveLength(4);
  render(<ChannelStorageDetails entity={component} model={model} onSelect={vi.fn()} />);
  expect(screen.getByText('READ declared by the owner')).toBeDefined();
  expect(screen.getAllByText('WRITE declared by the owner')).toHaveLength(2);
  expect(screen.getByText(/the store's access mode is not admission authority/i)).toBeDefined();
});

it('resolves a connection subject only against the full identity inventory and exact owner UID', () => {
  const model = fixture();
  const owner = model.entities[3];
  const subject = responsibilitySubjects(owner).find((item) => item.label.endsWith(': claim-a'))?.subject;
  expect(resolveAssociationSubject(model, subject)?.path).toBe('storageClaims[0]');
  (owner.raw.storageClaims as unknown[]).reverse();
  expect(resolveAssociationSubject(model, subject)?.path).toBe('storageClaims[2]');
  owner.raw.communications = [{ connectionId: { value: 'claim-a' } }];
  expect(resolveAssociationSubject(model, subject)).toBeUndefined();
  owner.raw.communications = [];
  owner.uid = 'replacement';
  expect(resolveAssociationSubject(model, subject)).toBeUndefined();
});

it('shows an unavailable attachment when the pinned store UID no longer matches', () => {
  const model = fixture();
  model.entities[2].uid = 'replacement-store';
  render(<ChannelStorageDetails entity={model.entities[3]} model={model} onSelect={vi.fn()} />);
  expect(screen.getAllByText('Unavailable exact attachment').length).toBeGreaterThan(0);
});

it.each(['duplicate attachment', 'missing attachment identity', 'group collision'] as const)(
  'does not hydrate an attachment with %s in the target owner',
  (mode) => {
    const model = fixture();
    const owner = model.entities[3];
    const store = model.entities[2];
    const rendered = render(<ChannelStorageDetails entity={owner} model={model} onSelect={vi.fn()} />);
    expect(screen.queryByText('Unavailable exact attachment')).toBeNull();
    if (mode === 'group collision') {
      store.raw.metadata = [
        {
          '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation',
          groups: [{ groupId: { value: 'attachment-a' }, items: [] }],
        },
      ];
    } else {
      const attachments = store.raw.attachments as unknown[];
      attachments.push(mode === 'duplicate attachment' ? attachments[0] : {});
    }
    rendered.rerender(<ChannelStorageDetails entity={owner} model={model} onSelect={vi.fn()} />);
    expect(screen.getByText('Unavailable exact attachment')).toBeDefined();
  },
);

it('is mounted by the Inspector for Channel, Component and DataProduct owners', () => {
  const model = fixture();
  for (const index of [0, 3, 4]) {
    cleanup();
    render(<Inspector entity={model.entities[index]} model={model} onSelect={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getByRole('heading', { name: 'Channel links and storage claims' })).toBeDefined();
  }
});

it.each([
  undefined,
  null,
  {},
  { connection: {} },
  { connection: { id: { value: 'link-a' } } },
  { connection: { owner: { kind: 'ENTITY_KIND_CHANNEL' }, ownerUid: 'hub-uid', id: { value: 'link-a' } } },
])('does not crash or substitute a selected association for a partial subject %j', (subject) => {
  const model = fixture();
  const context = buildModel([
    { document: { id: id('context'), title: { value: 'Context' }, subjects: [{ subject }] } },
  ]).entities[0];
  expect(resolveAssociationSubject(model, subject)).toBeUndefined();
  render(<ChannelStorageDetails entity={context} model={model} onSelect={vi.fn()} />);
  expect(screen.queryByRole('heading', { name: 'Selected associations in the loaded model' })).toBeNull();
});

const protoSources = import.meta.glob('../../../../../proto/trogonatlas/{eventmodel,type}/v1alpha1/*.proto', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;
const wireRoot = new protobuf.Root();
for (const wellKnown of ['any', 'duration', 'struct'])
  wireRoot.addJSON(
    (protobuf.common.get(`google/protobuf/${wellKnown}.proto`) as { nested: Record<string, protobuf.AnyNestedObject> })
      .nested,
  );
for (const source of Object.values(protoSources)) protobuf.parse(source, wireRoot);
wireRoot.resolveAll();
const entityType = wireRoot.lookupType('trogonatlas.eventmodel.v1alpha1.Entity');

function wireEntity(payload: Record<string, unknown>) {
  const packed = entityType.encode(entityType.fromObject(payload)).finish();
  return entityType.toObject(entityType.decode(packed), { longs: String, enums: String, bytes: String });
}

it('retains Channel and storage declarations through the real wire codec into Inspector projection', () => {
  const source = fixture();
  source.entities[0].raw.links = [
    { ...link('link-a'), parameterValues: { tenant: '', region: 'eu' } },
    link('link-b'),
    link('link-c', 'CHANNEL_LINK_KIND_ROUTES_TO'),
  ];
  source.entities[4].raw.inputs = [{ portId: { value: 'port-a' }, name: { value: 'Rows' } }];
  const rows = source.entities.map((owner) =>
    wireEntity({
      [owner.kind]: owner.raw,
      system: { uid: owner.uid ?? 'product-uid' },
    }),
  );
  rows.push(
    wireEntity({
      externalSystem: {
        id: id('external'),
        title: 'External',
        storageClaims: [claim('external-claim', 'STORAGE_DIRECTION_READ')],
      },
      system: { uid: 'external-uid' },
    }),
  );
  const model = buildModel(rows);
  const links = channelLinks(model.entities[0]);
  expect(links.map((item) => [item.connectionId, item.kind])).toEqual([
    ['link-a', 'COMPOSES'],
    ['link-b', 'COMPOSES'],
    ['link-c', 'ROUTES_TO'],
  ]);
  expect(links[0].target?.id.version).toBe('18446744073709551615');
  expect(links[0].parameterValues).toEqual([
    ['tenant', ''],
    ['region', 'eu'],
  ]);
  const claims = storageClaims(model.entities[3]);
  expect(claims.map((item) => item.direction)).toEqual(['READ', 'WRITE', 'WRITE']);
  expect(claims[0].attachment).toEqual({
    owner: { kind: 'ENTITY_KIND_DATA_STORE', id: id('warehouse') },
    ownerUid: 'store-uid',
    id: 'attachment-a',
  });
  expect(claims[1].attachment).toBeUndefined();
  expect(storageClaims(model.entities[4])[0].portId).toBe('port-a');
  expect(storageClaims(model.entities[5])[0].connectionId).toBe('external-claim');
  expect(storageClaims(model.entities[5])[0].store?.id.version).toBe('18446744073709551615');
  expect(resolveAssociationSubject(model, selection(model.entities[4], 'product-claim'))?.connectionId).toBe(
    'product-claim',
  );
  const select = vi.fn();
  const { container } = render(
    <Inspector entity={model.entities[0]} model={model} onSelect={select} onClose={vi.fn()} />,
  );
  expect(screen.getAllByText('COMPOSES declared by the owner')).toHaveLength(2);
  expect(screen.getByText('ROUTES_TO declared by the owner')).toBeDefined();
  expect(screen.getByText('tenant=')).toBeDefined();
  expect(screen.getByText('region=eu')).toBeDefined();
  expect(container.querySelector('script')).toBeNull();
  fireEvent.click(screen.getAllByRole('button', { name: 'shop/inner@18446744073709551615' })[0]);
  expect(select).toHaveBeenCalledWith(entityKey('channel', id('inner')));
  cleanup();
  render(<Inspector entity={model.entities[3]} model={model} onSelect={select} onClose={vi.fn()} />);
  expect(screen.getByText('READ declared by the owner')).toBeDefined();
  expect(screen.queryByText('Unavailable exact attachment')).toBeNull();
  cleanup();
  render(<Inspector entity={model.entities[5]} model={model} onSelect={select} onClose={vi.fn()} />);
  expect(screen.getByText('READ declared by the owner')).toBeDefined();
});

it.each(['claims', 'routes', 'triggers', 'fieldSelections'])(
  'refuses a composed storage selector that collides with messageClaims.%s',
  (field) => {
    const model = fixture();
    const owner = model.entities[3];
    const subject = selection(owner, 'claim-a');
    expect(resolveAssociationSubject(model, subject)?.connectionId).toBe('claim-a');
    owner.raw.messageClaims = { [field]: [{ connectionId: { value: 'claim-a' } }] };
    expect(resolveAssociationSubject(model, subject)).toBeUndefined();
  },
);

it('refuses a composed product storage selector that collides with a resource association', () => {
  const model = fixture();
  const owner = model.entities[4];
  owner.uid = 'product-uid';
  const subject = selection(owner, 'product-claim');
  expect(resolveAssociationSubject(model, subject)?.connectionId).toBe('product-claim');
  owner.raw.resourceAssociations = [{ connectionId: { value: 'product-claim' } }];
  expect(resolveAssociationSubject(model, subject)).toBeUndefined();
});

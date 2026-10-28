import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { buildModel, entityKey } from '@/lib/model';
import {
  channelLinks,
  incomingChannelLinks,
  incomingStorageClaims,
  resolveAssociationSubject,
  storageClaims,
} from '@/lib/channel-storage';
import { responsibilitySubjects } from '@/lib/responsibility';
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
      channel: { id: id('hub'), title: 'Hub', links: [link('link-a'), link('link-b'), link('link-c', 'CHANNEL_LINK_KIND_ROUTES_TO')] },
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
  const subject = responsibilitySubjects(owner)[2]?.subject;
  expect(resolveAssociationSubject(model, subject)?.path).toBe('storageClaims[1]');
  (owner.raw.storageClaims as unknown[]).reverse();
  expect(resolveAssociationSubject(model, subject)?.path).toBe('storageClaims[1]');
  owner.raw.communications = [{ connectionId: { value: 'claim-b' } }];
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

it('is mounted by the Inspector for Channel, Component and DataProduct owners', () => {
  const model = fixture();
  for (const index of [0, 3, 4]) {
    cleanup();
    render(<Inspector entity={model.entities[index]} model={model} onSelect={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getByRole('heading', { name: 'Channel links and storage claims' })).toBeDefined();
  }
});

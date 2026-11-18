import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildModel, wireKind, type EntityKind, type Model } from '@/lib/model';
import { ArchitectureResourceDetails } from './ArchitectureResourceDetails';

const id = (slug: string, version = '1') => ({ namespace: 'orders', slug, version });
const connectionId = (value: string) => ({ value });

function fixture(authoritative?: boolean, ownerUid = 'automation-owner') {
  return buildModel([
    { readModel: { id: id('pending', '2'), title: 'Pending' } },
    { processor: { id: id('notify'), title: 'Notify' } },
    {
      system: { uid: 'automation-owner' },
      automationSlice: {
        id: id('observe'),
        title: 'Observe',
        processor: { processor: { id: id('notify') }, connectionId: connectionId('processor-edge') },
        sourceReadModels: ['observation-a', 'observation-b'].map((value) => ({
          readModel: { id: id('pending', '2') },
          connectionId: connectionId(value),
        })),
      },
    },
    {
      channel: {
        id: id('notifications'),
        title: 'Notifications',
        address: {
          template: 'orders.{region}',
          parameters: [
            { name: { value: 'region' }, default: '', allowedValues: { values: ['', 'west'] }, examples: ['west'] },
          ],
        },
        protocols: [{ value: 'nats' }],
        deliveryGuarantee: 'DELIVERY_GUARANTEE_AT_LEAST_ONCE',
        bindings: ['observation-a', 'observation-b'].map((value) => ({
          connectionId: connectionId(`binding-${value}`),
          observation: {
            owner: { kind: 'ENTITY_KIND_AUTOMATION_SLICE', id: id('observe') },
            ownerUid,
            id: connectionId(value),
          },
          parameterValues: { region: 'west' },
        })),
      },
    },
    {
      dataStore: {
        id: id('pending-store'),
        title: 'Pending store',
        storeClass: { known: 'KNOWN_DATABASE' },
        technology: { value: 'PostgreSQL' },
        authoritative,
        accessMode: 'ACCESS_MODE_READ_WRITE',
        residency: { policy: { value: 'Regional policy' }, regions: { values: [{ value: 'west' }] } },
        retention: { policy: { value: 'Retain for a day' }, duration: { seconds: '86400' } },
        attachments: [{ connectionId: connectionId('attachment-a'), readModel: { id: id('pending', '2') } }],
      },
    },
  ]);
}

function resource(model: Model, kind: EntityKind) {
  const entity = model.byKind.get(kind)?.[0];
  if (!entity) throw new Error('Resource fixture missing');
  return entity;
}

afterEach(cleanup);

describe('architecture resource review', () => {
  it('normalizes resource envelopes and diagnostic wire kinds without adding behavioral slices', () => {
    const model = fixture(false);
    expect(model.byKind.get('channel')).toHaveLength(1);
    expect(model.byKind.get('dataStore')).toHaveLength(1);
    expect(wireKind('ENTITY_KIND_CHANNEL')).toBe('channel');
    expect(wireKind('ENTITY_KIND_DATA_STORE')).toBe('dataStore');
    expect(model.slices).toHaveLength(1);
    expect(model.byKind.get('dataStore')?.[0].raw.authoritative).toBe(false);
    expect(fixture().byKind.get('dataStore')?.[0].raw.authoritative).toBeUndefined();
  });

  it('shows distinct IDs for repeated endpoint observations and resolves their exact incarnation', () => {
    const model = fixture();
    render(<ArchitectureResourceDetails entity={resource(model, 'channel')} model={model} onSelect={vi.fn()} />);
    expect(screen.getByText('orders.{region}')).toBeDefined();
    expect(screen.getByText('nats')).toBeDefined();
    expect(screen.getByText('at least once')).toBeDefined();
    for (const suffix of ['a', 'b']) {
      expect(screen.getByText(`observation-${suffix}`)).toBeDefined();
      expect(screen.getByText(`binding-observation-${suffix}`)).toBeDefined();
    }
    expect(screen.getAllByText('automation-owner')).toHaveLength(2);
    expect(screen.getAllByText('orders/pending@2 → orders/notify@1')).toHaveLength(2);
  });

  it('does not derive endpoints from a recreated observation owner', () => {
    const model = fixture(undefined, 'retired-owner');
    render(<ArchitectureResourceDetails entity={resource(model, 'channel')} model={model} onSelect={vi.fn()} />);
    expect(screen.getAllByText('Not resolved in the loaded model')).toHaveLength(2);
    expect(screen.queryByText('orders/pending@2 → orders/notify@1')).toBeNull();
  });

  it.each([
    [false, 'No'],
    [true, 'Yes'],
    [undefined, 'Unknown'],
  ] as const)('preserves authoritative presence for %s', (authoritative, label) => {
    const model = fixture(authoritative);
    const onSelect = vi.fn();
    render(<ArchitectureResourceDetails entity={resource(model, 'dataStore')} model={model} onSelect={onSelect} />);
    expect(screen.getByText(label)).toBeDefined();
    expect(screen.getByText('PostgreSQL')).toBeDefined();
    expect(screen.getByText('Regional policy')).toBeDefined();
    expect(screen.getByText('86400 seconds')).toBeDefined();
    expect(screen.getByText('attachment-a')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'orders/pending@2' }));
    expect(onSelect).toHaveBeenCalledWith('readModel:orders/pending@2');
  });
});

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { buildModel } from '@/lib/model';
import { SchemaBusinessDetails } from './SchemaBusinessDetails';

const version = '9007199254740993';
const id = (slug: string, entityVersion = version) => ({ namespace: 'shop', slug, version: entityVersion });

function stringField(name: string, repeated = false) {
  return { name, repeated, type: { string: {} } };
}

function schemaRefField(name: string, target: ReturnType<typeof id>) {
  return { name, type: { ref: { ref: { schema: { id: target } } } } };
}

function orderFields() {
  return [
    { name: 'customer', repeated: false, type: { object: { fields: [stringField('id')] } } },
    { name: 'lines', repeated: true, type: { object: { fields: [stringField('sku')] } } },
    schemaRefField('product_ref', id('product', '1')),
  ];
}

function model() {
  return buildModel([
    {
      schema: {
        id: id('order', '1'),
        title: 'Order',
        fields: orderFields(),
        identifiers: [
          {
            connectionId: { value: '11111111-1111-4111-8111-111111111111' },
            property: { segments: [{ name: 'customer', repeated: false }, { name: 'id', repeated: false }] },
            role: 'BUSINESS_IDENTIFIER_ROLE_PRIMARY',
          },
        ],
        propertyRelationships: [
          {
            connectionId: { value: '22222222-2222-4222-8222-222222222222' },
            source: { segments: [{ name: 'product_ref', repeated: false }] },
            targetEntity: { id: id('product', version) },
            label: { value: 'hasMany' },
            targetProperty: { segments: [{ name: 'sku', repeated: false }] },
          },
        ],
      },
      system: { uid: 'order-incarnation' },
    },
    {
      schema: {
        id: id('product', version),
        title: 'Product',
        fields: [stringField('name')],
      },
    },
  ]);
}

afterEach(cleanup);

it('shows unset aggregate intent as unspecified and keeps a type ref beside a relationship', () => {
  const loaded = model();
  render(<SchemaBusinessDetails entity={loaded.entities[0]} model={loaded} />);
  expect(screen.getByText(/Loaded-model inspection/)).toBeDefined();
  expect(screen.getByText(/Aggregate root intent:/).textContent).toContain('unspecified');
  expect(screen.getByText('primary')).toBeDefined();
  expect(screen.getByText('customer / id')).toBeDefined();
  expect(screen.getByText('11111111-1111-4111-8111-111111111111')).toBeDefined();
  expect(screen.getByText('path resolves in this loaded schema')).toBeDefined();
  expect(screen.getByText('fields[2].ref shop/product@1')).toBeDefined();
  expect(screen.getByText('hasMany')).toBeDefined();
  expect(screen.getByText(`shop/product@${version}`)).toBeDefined();
  expect(screen.getByText('target path is unresolved in the loaded model')).toBeDefined();
  expect(screen.getByText(/separate facts/)).toBeDefined();
});

it('distinguishes a stored false aggregate intent from an omitted one', () => {
  const present = buildModel([
    { schema: { id: id('order', '1'), title: 'Order', aggregateRoot: false, fields: [] } },
  ]);
  const { rerender } = render(<SchemaBusinessDetails entity={present.entities[0]} model={present} />);
  expect(screen.getByText(/Aggregate root intent:/).textContent).toContain('false');
  const storedTrue = buildModel([
    { schema: { id: id('order', '1'), title: 'Order', aggregateRoot: true, fields: [] } },
  ]);
  rerender(<SchemaBusinessDetails entity={storedTrue.entities[0]} model={storedTrue} />);
  expect(screen.getByText(/Aggregate root intent:/).textContent).toContain('true');
});

it('does not inspect a non-schema owner', () => {
  const loaded = buildModel([{ event: { id: id('order-placed', '1'), title: 'Order placed' } }]);
  render(<SchemaBusinessDetails entity={loaded.entities[0]} model={loaded} />);
  expect(screen.queryByText(/Loaded-model inspection/)).toBeNull();
});

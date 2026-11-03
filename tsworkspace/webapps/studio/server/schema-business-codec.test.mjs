// @vitest-environment jsdom
import { createRequire } from 'node:module';
import path from 'node:path';
import { createElement } from 'react';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { buildModel } from '../src/lib/model.ts';
import { SchemaBusinessDetails } from '../src/components/SchemaBusinessDetails.tsx';

const require = createRequire(import.meta.url);
const protoLoader = require('@grpc/proto-loader');
const version = '9007199254740993';
const protoRoot = path.resolve(import.meta.dirname, '../../../../proto');
const definitions = protoLoader.loadSync(path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'), {
  keepCase: false,
  longs: String,
  enums: String,
  defaults: false,
  oneofs: false,
  includeDirs: [protoRoot],
});
const entityCodec = definitions['trogonatlas.eventmodel.v1alpha1.Entity'];

function throughBridge(input) {
  return entityCodec.deserialize(entityCodec.serialize(input));
}

const productId = { namespace: 'shop', slug: 'product', version };
const orderFields = [
  {
    name: 'customer',
    repeated: false,
    type: { object: { fields: [{ name: 'id', repeated: false, type: { string: {} } }] } },
  },
  {
    name: 'lines',
    repeated: true,
    type: { object: { fields: [{ name: 'sku', repeated: false, type: { string: {} } }] } },
  },
  { name: 'product_ref', type: { ref: { ref: { schema: { id: productId } } } } },
];

function orderWire(aggregateRoot) {
  const schema = {
    id: { namespace: 'shop', slug: 'order', version },
    title: 'Order',
    fields: orderFields,
    identifiers: [
      {
        connectionId: { value: '11111111-1111-4111-8111-111111111111' },
        property: {
          segments: [
            { name: 'customer', repeated: false },
            { name: 'id', repeated: false },
          ],
        },
        role: 'BUSINESS_IDENTIFIER_ROLE_PRIMARY',
      },
    ],
    propertyRelationships: [
      {
        connectionId: { value: '22222222-2222-4222-8222-222222222222' },
        source: { segments: [{ name: 'product_ref', repeated: false }] },
        targetEntity: { id: productId },
        label: { value: 'hasMany' },
      },
      {
        connectionId: { value: '33333333-3333-4333-8333-333333333333' },
        source: {
          segments: [
            { name: 'lines', repeated: true },
            { name: 'sku', repeated: false },
          ],
        },
        targetEntity: { id: productId },
        targetProperty: { segments: [{ name: 'sku', repeated: false }] },
        label: { value: 'belongsTo' },
      },
      {
        connectionId: { value: '44444444-4444-4444-8444-444444444444' },
        source: { segments: [{ name: 'customer', repeated: false }] },
        targetEntity: { id: productId },
        targetProperty: { segments: [{ name: 'missing', repeated: false }] },
      },
    ],
  };
  if (aggregateRoot !== undefined) schema.aggregateRoot = aggregateRoot;
  return { schema, system: { uid: 'order-incarnation' } };
}

function modelFrom(wire) {
  const model = buildModel(wire.map((item) => throughBridge(item)));
  const entity = model.entities.find((item) => item.id.slug === 'order');
  if (!entity) throw new Error('order missing after codec');
  return { entity, model };
}

function show(entity, model) {
  render(createElement(SchemaBusinessDetails, { entity, model }));
}

afterEach(cleanup);

it('decodes omitted aggregate intent and selector facts through the studio bridge codec', () => {
  const decoded = throughBridge(orderWire());
  expect(decoded.schema.aggregateRoot).toBeUndefined();
  expect(decoded.schema.id.version).toBe(version);
  const { entity, model } = modelFrom([
    orderWire(),
    {
      schema: {
        id: productId,
        title: 'Product',
        fields: [{ name: 'sku', repeated: false, type: { string: {} } }],
      },
    },
  ]);
  expect(entity.id.version).toBe(version);
  expect(entity.raw.aggregateRoot).toBeUndefined();
  show(entity, model);
  expect(screen.getByText(/Aggregate root intent:/).textContent).toContain('unspecified');
  expect(screen.getByText('primary')).toBeDefined();
  expect(screen.getByText('customer / id')).toBeDefined();
  expect(screen.getByText('11111111-1111-4111-8111-111111111111')).toBeDefined();
  expect(screen.getByText('path resolves in this loaded schema')).toBeDefined();
  expect(screen.getByText(`fields[2].ref shop/product@${version}`)).toBeDefined();
  expect(screen.getByText('hasMany')).toBeDefined();
  expect(screen.getByText('entity only')).toBeDefined();
  expect(screen.getByText('target path resolves in the loaded model')).toBeDefined();
  expect(screen.getByText('target path is unresolved in the loaded model')).toBeDefined();
  expect(screen.getAllByText(`shop/product@${version}`)).toHaveLength(3);
  expect(screen.getByText(/separate facts/)).toBeDefined();
});

it('keeps a stored false aggregate intent distinct from true after the same codec', () => {
  const absent = modelFrom([orderWire()]);
  const present = modelFrom([orderWire(false)]);
  const storedTrue = modelFrom([orderWire(true)]);
  expect(absent.entity.raw.aggregateRoot).toBeUndefined();
  expect(present.entity.raw.aggregateRoot).toBe(false);
  expect(storedTrue.entity.raw.aggregateRoot).toBe(true);
  show(present.entity, present.model);
  expect(screen.getByText(/Aggregate root intent:/).textContent).toContain('false');
  cleanup();
  show(storedTrue.entity, storedTrue.model);
  expect(screen.getByText(/Aggregate root intent:/).textContent).toContain('true');
});

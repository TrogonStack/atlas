import path from 'node:path';
import protobuf from 'protobufjs';
import protoLoader from '@grpc/proto-loader';
import { expect, it } from 'vitest';
import { buildModel } from '../src/lib/model.ts';

const protoRoot = path.resolve(import.meta.dirname, '../../../../proto');
const servicePath = path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto');
const definitions = protoLoader.loadSync(servicePath, { keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [protoRoot] });
const entityCodec = definitions['trogonatlas.eventmodel.v1alpha1.Entity'];
const id = { namespace: 'catalog', slug: 'order', version: '18446744073709551615' };
const property = { segments: [{ name: 'id', repeated: false }] };

it('preserves native Schema business presence and catalog strings through the Studio codec', () => {
  const schema = {
    id, title: 'Order', aggregateRoot: false,
    fields: [{ name: 'id', type: { string: {} } }],
    identifiers: [{ connectionId: { value: 'identifier' }, property, role: 'BUSINESS_IDENTIFIER_ROLE_PRIMARY', doc: { value: 'Café\r\n' } }],
    propertyRelationships: [{ connectionId: { value: 'relationship' }, source: property, targetEntity: { id }, targetProperty: property, label: { value: 'same entity' }, doc: { value: '<script>inert()</script>' } }],
  };
  const input = { schema, system: { uid: 'schema-incarnation' } };
  const decoded = entityCodec.deserialize(entityCodec.serialize(input));
  expect(decoded).toEqual(input);
  const model = buildModel([decoded]);
  expect(model.entities[0].raw.aggregateRoot).toBe(false);
  expect(model.entities[0].raw.propertyRelationships[0].targetEntity.id.version).toBe(id.version);
  const absent = entityCodec.deserialize(entityCodec.serialize({ schema: { id, title: 'Unspecified' } }));
  expect('aggregateRoot' in absent.schema).toBe(false);
});

it('keeps graph step tags and checked business status distinct in the actual Reference descriptor', () => {
  const root = new protobuf.Root();
  root.resolvePath = (_origin, target) => path.resolve(protoRoot, target);
  root.loadSync(servicePath);
  const descriptor = root.lookupType('trogonatlas.api.eventmodel.v1alpha1.Reference');
  expect(descriptor.fields.stepId.id).toBe(10);
  expect(descriptor.fields.targetStepId.id).toBe(11);
  expect(descriptor.fields.businessPropertyStatus.id).toBe(12);
  const codec = definitions['trogonatlas.api.eventmodel.v1alpha1.Reference'];
  const reference = { from: { kind: 'ENTITY_KIND_SCHEMA', id }, to: { kind: 'ENTITY_KIND_SCHEMA', id }, fromField: 'property_relationships[0].target_entity', fromUid: 'schema-incarnation', connectionId: { value: 'relationship' }, stepId: { value: 'step' }, targetStepId: { value: 'next-step' }, businessPropertyStatus: 'BUSINESS_PROPERTY_REFERENCE_STATUS_PROPERTY_RESOLVED' };
  expect(codec.deserialize(codec.serialize(reference))).toEqual(reference);
});

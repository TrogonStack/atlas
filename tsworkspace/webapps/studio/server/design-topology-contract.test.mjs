import path from 'node:path';
import protobuf from 'protobufjs';
import protoLoader from '@grpc/proto-loader';
import { describe, expect, it } from 'vitest';
import { validateCatalogRequest } from './decision-document-request.mjs';

const protoRoot = path.resolve(import.meta.dirname, '../../../../proto');
const servicePath = path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto');
const root = new protobuf.Root();
root.resolvePath = (_origin, target) => path.resolve(protoRoot, target);
root.loadSync(servicePath);
const definitions = protoLoader.loadSync(servicePath, { keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [protoRoot] });
const id = { namespace: 'catalog', slug: 'checkout-flow', version: '9007199254740993' };
const ref = { id };
const subject = { entity: { kind: 'ENTITY_KIND_COMPONENT', id } };
function type(rpc) { return root.lookupType(`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`); }
function roundTrip(rpc, input) {
  validateCatalogRequest(type(rpc), input);
  const service = definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService'];
  const method = Object.values(service).find(item => item.path.endsWith(`/${rpc}`));
  const codec = definitions[`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`];
  return codec.deserialize(method.requestSerialize(input));
}
describe('design topology actual serializer boundary', () => {
  it('preserves exact references, subflow switch and depth', () => {
    expect(roundTrip('GetDesignGraphDetail', { graph: ref })).toEqual({ graph: ref });
    expect(roundTrip('ListIncomingDesignGraphSubflows', { graph: ref })).toEqual({ graph: ref });
    expect(roundTrip('ListSubjectDesignGraphs', { subject })).toEqual({ subject });
    for (const followSubflows of [false, true])
      expect(roundTrip('TraverseDesignGraphTopology', { root: ref, followSubflows, maxDepth: 3 })).toEqual({ root: ref, followSubflows, maxDepth: 3 });
  });
  it.each([
    ['GetDesignGraphDetail', { graph: ref, surprise: true }],
    ['GetDesignGraphDetail', { graph: { id: { ...id, version: 9007199254740992 } } }],
    ['TraverseDesignGraphTopology', { root: ref, followSubflows: 'true' }],
    ['TraverseDesignGraphTopology', { root: ref, maxDepth: -1 }],
    ['TraverseDesignGraphTopology', { root: ref, maxDepth: '3' }],
    ['ListSubjectDesignGraphs', { subject: { entity: { kind: true, id } } }],
    ['ListIncomingDesignGraphSubflows', { graph: { id: { ...id, version: '18446744073709551616' } } }],
  ])('rejects malformed %s before coercion can erase intent', (rpc, input) => {
    expect(() => roundTrip(rpc, input)).toThrow(/expected|unknown/);
  });
});

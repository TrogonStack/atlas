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

it('preserves integrated SystemGroup topology and repeated Schema/DesignGraph members through actual service codecs', () => {
  const service = definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService'];
  const method = (rpc) => Object.values(service).find((item) => item.path.endsWith(`/${rpc}`));
  const groupId = { ...id, slug: 'platform', version: '18446744073709551615' };
  const target = { systemGroup: { id: groupId } };
  const graph = { designGraph: { id, title: { value: 'Graph' }, steps: [{ stepId: { value: '01930000-0000-7000-8000-000000000001' }, title: { value: 'Group step' }, target }] } };
  const members = [
    { connectionId: { value: '01930000-0000-7000-8000-000000000002' }, member: { schema: ref } },
    { connectionId: { value: '01930000-0000-7000-8000-000000000003' }, member: { designGraph: ref } },
    { connectionId: { value: '01930000-0000-7000-8000-000000000004' }, member: { designGraph: ref } },
  ];
  for (const entity of [graph, { systemGroup: { id: groupId, title: 'Platform', scope: 'SYSTEM_GROUP_SCOPE_EXTERNAL', members } }, { domain: { id: groupId, members } }]) {
    const input = { entity };
    const codec = definitions['trogonatlas.api.eventmodel.v1alpha1.PutEntityRequest'];
    expect(codec.deserialize(method('PutEntity').requestSerialize(input))).toEqual(input);
  }
  const input = { entity: graph, targets: [{ target, state: 'CATALOG_RESOLUTION_STATE_RESOLVED', owner: { owner: { kind: 'ENTITY_KIND_SYSTEM_GROUP', id: groupId }, uid: 'native-group' } }] };
  const codec = definitions['trogonatlas.api.eventmodel.v1alpha1.GetDesignGraphDetailResponse'];
  expect(codec.deserialize(method('GetDesignGraphDetail').responseSerialize(input))).toEqual(input);
});

import path from 'node:path';
import protobuf from 'protobufjs';
import protoLoader from '@grpc/proto-loader';
import { describe, expect, it } from 'vitest';
import { validateSourceRequest } from './source-request.mjs';

const protoRoot = path.resolve(import.meta.dirname, '../../../../proto');
const servicePath = path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto');
const root = new protobuf.Root();
root.resolvePath = (_origin, target) => path.resolve(protoRoot, target);
root.loadSync(servicePath);
const definitions = protoLoader.loadSync(servicePath, { keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [protoRoot] });
function roundTrip(rpc, input) {
  validateSourceRequest(root.lookupType(`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`), input);
  const method = Object.values(definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService']).find(item => item.path.endsWith(`/${rpc}`));
  return definitions[`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`].deserialize(method.requestSerialize(input));
}

describe('source gateway actual serializer boundary', () => {
  it.each([
    ['GetSourceSnapshot', { registrationId: 'r', snapshotId: 's', includeBytes: false }],
    ['GetSourceSnapshot', { registrationId: 'r', snapshotId: 's', includeBytes: true }],
    ['ListSourceRegistrations', { administrationNamespace: 'models', pageSize: 500, pageToken: 'opaque' }],
    ['GetSourceOverlay', { registrationId: 'r', snapshotId: 's', pageSize: 100, pageToken: 'opaque' }],
    ['ListSourceRunResults', { registrationId: 'r', runId: 'run', pageSize: 100, pageToken: 'opaque' }],
    ['ListSourceRunOperations', { registrationId: 'r', runId: 'run', pageSize: 100, pageToken: 'opaque' }],
  ])('preserves %s exact IDs, explicit booleans and cursors', (rpc, input) => {
    expect(roundTrip(rpc, input)).toEqual(input);
  });
  it.each([
    ['GetSourceSnapshot', { registrationId: 'r', snapshotId: 's', includeBytes: 'false' }],
    ['GetSourceSnapshot', { registrationId: 'r', includeBytes: null }],
    ['GetSourceSnapshot', { registrationId: ['r'] }],
    ['GetSourceSnapshot', { registration_id: 'r' }],
    ['GetSourceSnapshot', { registrationId: 'r', surprise: true }],
    ['ListSourceRuns', { registrationId: 'r', pageSize: '100' }],
    ['ListSourceRuns', { registrationId: 'r', pageSize: -1 }],
    ['ListSourceRuns', { registrationId: 'r', pageSize: 4294967296 }],
    ['ListSourceRuns', { registrationId: 'r', pageToken: false }],
  ])('rejects malformed %s before serializer coercion', (rpc, input) => {
    expect(() => roundTrip(rpc, input)).toThrow('invalid source query request');
  });
  it('rejects unknown nested enums and ambiguous selectors before serialization', () => {
    const owner = { kind: 'ENTITY_KIND_READ_MODEL', id: { namespace: 'n', slug: 'x', version: '9007199254740993' } };
    const mapping = { source: { registrationId: 'r', category: 'native', key: 'x' }, selection: { entity: owner }, ownership: { none: {} } };
    expect(roundTrip('CreateSourceMapping', { mapping })).toEqual({ mapping });
    for (const selection of [{}, { entity: owner, document: { document: { id: owner.id } } }, { entity: { ...owner, kind: 'TYPO' } }, { entity: { ...owner, kind: 991 } }, { entity: { ...owner, id: { ...owner.id, version: 9007199254740992 } } }]) {
      expect(() => roundTrip('CreateSourceMapping', { mapping: { ...mapping, selection } })).toThrow('invalid source query request');
    }
  });
  it('preserves ordered repeated pointers in actual response serialization', () => {
    const codec = definitions['trogonatlas.api.eventmodel.v1alpha1.GetSourceOverlayResponse'];
    const entries = ['b', 'a'].map(occurrenceKey => ({ source: { registrationId: 'r', category: 'm', key: 'x' }, occurrenceKey, pointer: { kind: 'opaque', raw: '<script>same</script>' }, position: '9007199254740993', status: 'SOURCE_OVERLAY_STATUS_DEFERRED' }));
    const input = { overlay: { snapshotId: 's', revision: 'rev', branch: 'review', entries, incomplete: true }, nextPageToken: 'more' };
    expect(codec.deserialize(codec.serialize(input))).toEqual(input);
  });
});

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
const id = { namespace: 'catalog', slug: 'note', version: '9007199254740993' };
const ref = { id };
const subjects = [
  { entity: { kind: 'ENTITY_KIND_READ_MODEL', id } },
  { connection: { owner: { kind: 'ENTITY_KIND_AUTOMATION_SLICE', id }, ownerUid: 'incarnation', id: { value: 'occurrence-b' } } },
  { member: { specification: ref, ownerUid: 'spec-incarnation', kind: 'CONTRACT_MEMBER_KIND_ARTIFACT', memberId: { value: 'member-b' }, expectedDigest: { sha256: 'a'.repeat(64) } } },
  { port: { product: ref, ownerUid: 'product-incarnation', direction: 'DATA_PORT_DIRECTION_OUTPUT', portId: { value: 'port-b' } } },
];
const selections = [{ document: ref }, { anchor: { document: ref, ownerUid: 'doc-incarnation', anchorId: { value: 'anchor-b' } } }, { anchor: { document: ref, ownerUid: 'doc-incarnation', anchorId: { value: 'anchor-b' }, expectedDigest: { sha256: 'b'.repeat(64) } } }];
function type(rpc) { return root.lookupType(`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`); }
function roundTrip(rpc, input) {
  validateCatalogRequest(type(rpc), input);
  const service = definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService'];
  const method = Object.values(service).find(item => item.path.endsWith(`/${rpc}`));
  const codec = definitions[`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`];
  return codec.deserialize(method.requestSerialize(input));
}
describe('catalog actual serializer boundary', () => {
  it('preserves explicit page receipts through the actual response codec', () => {
    const service = definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService'];
    const method = Object.values(service).find(item => item.path.endsWith('/ListIncomingDocumentDeclarations'));
    const response = { scope: { revision: 'catalog-page-checked-v2:opaque', visited: [{ owner: { kind: 'ENTITY_KIND_DOCUMENT', id }, uid: 'owner-incarnation', etag: 'revision' }], pageReceipt: { checkedOwnerCount: '5000', totalDeclarations: '4097', startOffset: '500', endOffset: '1000' } } };
    expect(method.responseDeserialize(method.responseSerialize(response))).toEqual(response);
  });
  it('preserves exact amendment page context without enum coercion', () => {
    const input = { decision: ref, direction: 'DECISION_AMENDMENT_DIRECTION_INCOMING', pageSize: 1, pageToken: 'opaque-context' };
    expect(roundTrip('ListDecisionAmendments', input)).toEqual(input);
    expect(() => roundTrip('ListDecisionAmendments', { ...input, direction: 'TYPO' })).toThrow(/known enum/);
  });
  it('preserves exact selector alternatives, pins, optional status and explicit false', () => {
    for (const subject of subjects) {
      expect(roundTrip('ListSubjectDocuments', { subject })).toEqual({ subject });
      for (const status of [undefined, 'DECISION_STATUS_ACCEPTED']) {
        const input = { subject, ...(status === undefined ? {} : { status }) };
        expect(roundTrip('ListSubjectDecisions', input)).toEqual(input);
      }
    }
    for (const document of selections) for (const rpc of ['GetDocumentDetail', 'ListIncomingDocumentDeclarations']) expect(roundTrip(rpc, { document })).toEqual({ document });
    for (const followReplacement of [false, true]) expect(roundTrip('GetDecisionDetail', { decision: ref, followReplacement })).toEqual({ decision: ref, followReplacement });
    expect(roundTrip('ListIncomingDecisionDeclarations', { decision: ref })).toEqual({ decision: ref });
  });
  it.each([
    ['ListSubjectDecisions', { subject: subjects[0], status: 'TYPO' }],
    ['ListSubjectDecisions', { subject: subjects[0], status: 991 }],
    ['GetDecisionDetail', { decision: ref, followReplacement: 'false' }],
    ['GetDecisionDetail', { decision: ref, followReplacement: 0 }],
    ['GetDecisionDetail', { decision: ref, followReplacement: null }],
    ['GetDecisionDetail', { decision: ref, surprise: true }],
    ['GetDocumentDetail', { document: {} }],
    ['GetDocumentDetail', { document: { document: ref, anchor: selections[1].anchor } }],
    ['GetDocumentDetail', { document: { anchor: { ...selections[1].anchor, ownerUid: false } } }],
    ['GetDocumentDetail', { document: { anchor: { ...selections[1].anchor, expected_digest: { sha256: 'a'.repeat(64) } } } }],
    ['ListSubjectDocuments', { subject: { ...subjects[0], ...subjects[3] } }],
    ['ListSubjectDocuments', { subject: { entity: { kind: true, id } } }],
    ['ListSubjectDocuments', { subject: { entity: { kind: 'ENTITY_KIND_EVENT', id: { ...id, version: 9007199254740992 } } } }],
    ['ListSubjectDocuments', { subject: { entity: { kind: 'ENTITY_KIND_EVENT', id: { ...id, version: '18446744073709551616' } } } }],
  ])('rejects malformed %s before coercion can erase intent', (rpc, input) => {
    expect(() => roundTrip(rpc, input)).toThrow(/expected|unknown/);
  });
});

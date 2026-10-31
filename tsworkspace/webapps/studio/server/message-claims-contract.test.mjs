import path from 'node:path';
import protoLoader from '@grpc/proto-loader';
import { describe, expect, it } from 'vitest';
import { buildModel } from '../src/lib/model.ts';
import { displaySelector, incomingMessageClaims, messageClaimLeaves, resolveMessageClaimSubject } from '../src/lib/message-claims.ts';
import { responsibilitySubjects } from '../src/lib/responsibility.ts';

const protoRoot = path.resolve(import.meta.dirname, '../../../../proto');
const definitions = protoLoader.loadSync(path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'), {
  keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [protoRoot],
});
const entityCodec = definitions['trogonatlas.eventmodel.v1alpha1.Entity'];
const id = (slug) => ({ namespace: 'catalog', slug, version: '18446744073709551615' });
const message = { specification: { id: id('events') }, ownerUid: 'spec-incarnation', kind: 'CONTRACT_MEMBER_KIND_MESSAGE', memberId: { value: 'message-member' } };
const claims = () => ({
  claims: ['a', 'b'].map((value) => ({ connectionId: { value: `claim-${value}` }, direction: 'COMMUNICATION_DIRECTION_SEND', message })),
  routes: [
    { connectionId: { value: 'route-a' }, claimId: { value: 'claim-a' }, channel: { id: id('orders') }, parameterValues: { tenant: '', region: 'eu' }, deliveryMode: 'CLAIM_DELIVERY_MODE_PULL' },
    { connectionId: { value: 'route-b' }, claimId: { value: 'claim-a' }, channel: { id: id('orders') } },
  ],
  triggers: [{ connectionId: { value: 'trigger-a' }, claimId: { value: 'claim-b' }, message, condition: { value: 'Café\r\n<script>inert()</script>' } }],
  fieldSelections: [{
    connectionId: { value: 'selection-a' },
    claimId: { value: 'claim-a' },
    binding: { owner: { kind: 'ENTITY_KIND_SPECIFICATION', id: id('events') }, ownerUid: 'spec-incarnation', id: { value: 'binding-a' } },
    selectors: [{ path: { segments: ['customer', 'name'] } }, { opaque: { syntax: 'json-pointer', value: '/free' } }],
  }],
});
const roundTrip = (entity) => entityCodec.deserialize(entityCodec.serialize(entity));

describe('message claims actual protobuf codec feeding the Studio projection', () => {
  it.each(['component', 'externalSystem', 'domain'])('preserves every %s claim leaf exactly and projects it unmerged', (kind) => {
    const entity = { [kind]: { id: id('owner'), title: 'Owner', messageClaims: claims() }, system: { uid: 'owner-incarnation' } };
    const decoded = roundTrip(entity);
    expect(decoded).toEqual(entity);
    const route = decoded[kind].messageClaims.routes;
    expect(route[0].deliveryMode).toBe('CLAIM_DELIVERY_MODE_PULL');
    expect('deliveryMode' in route[1]).toBe(false);
    expect(route[0].parameterValues).toEqual({ tenant: '', region: 'eu' });
    expect(decoded[kind].messageClaims.claims.map((claim) => claim.message)).toEqual([message, message]);

    const model = buildModel([decoded, roundTrip({ channel: { id: id('orders'), title: 'Orders' }, system: { uid: 'channel-incarnation' } })]);
    const owner = model.entities.find((item) => item.kind === kind);
    const leaves = messageClaimLeaves(owner);
    expect(leaves.map((leaf) => leaf.connectionId)).toEqual(['claim-a', 'claim-b', 'route-a', 'route-b', 'trigger-a', 'selection-a']);
    expect(leaves.map((leaf) => leaf.path)).toContain('messageClaims.routes[1]');
    expect(leaves[3].body.deliveryMode).toBeUndefined();
    expect(leaves[2].body.parameterValues).toEqual({ tenant: '', region: 'eu' });
    expect(leaves[5].body.selectors.map(displaySelector)).toEqual([
      { kind: 'path', text: '["customer","name"]' },
      { kind: 'opaque', text: 'json-pointer: /free' },
    ]);
    const channel = model.entities.find((item) => item.kind === 'channel');
    expect(incomingMessageClaims(model, channel).map((leaf) => leaf.connectionId)).toEqual(['route-a', 'route-b']);
    const subject = responsibilitySubjects(owner).find((item) => item.label.endsWith(': route-b')).subject;
    expect(roundTrip({ team: { id: id('team'), responsibilities: [{ connectionId: { value: 'assignment' }, kind: 'RESPONSIBILITY_KIND_OWNER', subject }] } }).team.responsibilities[0].subject).toEqual(subject);
    expect(resolveMessageClaimSubject(model, subject)?.path).toBe('messageClaims.routes[1]');
  });
  it('shows a selector with no alternative as unavailable instead of guessing', () => {
    const body = claims();
    body.fieldSelections[0].selectors = [{}];
    const decoded = roundTrip({ domain: { id: id('owner'), messageClaims: body }, system: { uid: 'owner-incarnation' } });
    expect(displaySelector(decoded.domain.messageClaims.fieldSelections[0].selectors[0])).toEqual({ kind: 'unavailable' });
  });
});

it('round trips every Channel inverse authority and exact occurrence without collapsing pins', () => {
  const codec = definitions['trogonatlas.api.eventmodel.v1alpha1.ListChannelMessagesResponse'];
  const base = { owner: { kind: 'ENTITY_KIND_SPECIFICATION', id: id('events') }, ownerUid: 'spec-incarnation', message, channel: { id: id('orders') }, state: 'CATALOG_RESOLUTION_STATE_RESOLVED' };
  const body = {
    records: [
      { ...base, connectionId: { value: 'independent' }, authority: 'CHANNEL_MESSAGE_AUTHORITY_SPECIFICATION_DECLARATION', parameterValues: { tenant: '' } },
      { ...base, owner: { kind: 'ENTITY_KIND_COMPONENT', id: id('owner') }, ownerUid: 'owner-incarnation', connectionId: { value: 'route-a' }, authority: 'CHANNEL_MESSAGE_AUTHORITY_CLAIM_ROUTE', claim: { ...claims().claims[0], group: { value: 'Orders' } }, route: claims().routes[0], deliveryMode: 'CLAIM_DELIVERY_MODE_PULL' },
      { ...base, connectionId: { value: 'binding' }, authority: 'CHANNEL_MESSAGE_AUTHORITY_OPERATION_BINDING', operation: { ...message, kind: 'CONTRACT_MEMBER_KIND_OPERATION', memberId: { value: 'operation' } }, operationMessage: { connectionId: { value: 'operation-message' }, messageId: message.memberId, role: 'ROLE_REQUEST' } },
    ], nextPageToken: 'opaque', complete: false, incomplete: false, scopeDigest: 'checked-content', authorizedNamespaceCount: 1,
  };
  const decoded = codec.deserialize(codec.serialize(body));
  expect(decoded).toEqual(body);
  expect(decoded.records.map((row) => row.message.specification.id.version)).toEqual(Array(3).fill('18446744073709551615'));
  expect('deliveryMode' in decoded.records[0]).toBe(false);
  expect(decoded.records[1].claim.group.value).toBe('Orders');
  expect(decoded.records[2].operationMessage.connectionId.value).toBe('operation-message');
});

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { buildModel, entityKey, type Entity } from '@/lib/model';
import {
  incomingMessageClaims,
  messageClaimLeaves,
  resolveMessageClaimSubject,
  type MessageClaimLeaf,
} from '@/lib/message-claims';
import { resolveCommunicationSubject } from '@/lib/communication-intent';
import { responsibilitySubjects } from '@/lib/responsibility';
import { Inspector } from './Inspector';
import { MessageClaimsDetails } from './MessageClaimsDetails';

vi.mock('@/lib/useEntityHomes', () => ({
  useEntityHomes: () => ({ homes: new Map(), error: undefined, retry: vi.fn() }),
}));
vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: () => <button type="button">Copy context</button>,
}));

const id = (slug: string, version = '18446744073709551615') => ({ namespace: 'shop', slug, version });
const message = (member = 'message-a') => ({
  specification: { id: id('events') },
  ownerUid: 'spec-uid',
  kind: 'CONTRACT_MEMBER_KIND_MESSAGE',
  memberId: { value: member },
});
const claims = () => ({
  claims: [
    {
      connectionId: { value: 'claim-a' },
      direction: 'COMMUNICATION_DIRECTION_SEND',
      message: message(),
      group: { value: 'Orders' },
    },
    { connectionId: { value: 'claim-b' }, direction: 'COMMUNICATION_DIRECTION_SEND', message: message() },
    { connectionId: { value: 'claim-c' }, direction: 'COMMUNICATION_DIRECTION_RECEIVE', message: message('message-b') },
  ],
  routes: [
    {
      connectionId: { value: 'route-a' },
      claimId: { value: 'claim-a' },
      channel: { id: id('orders') },
      parameterValues: { tenant: '', region: 'eu' },
      deliveryMode: 'CLAIM_DELIVERY_MODE_PUSH_PULL',
    },
    { connectionId: { value: 'route-b' }, claimId: { value: 'claim-a' }, channel: { id: id('orders') } },
  ],
  triggers: [
    {
      connectionId: { value: 'trigger-a' },
      claimId: { value: 'claim-c' },
      message: message(),
      condition: { value: '<script>inert()</script>\nwhen approved' },
    },
    { connectionId: { value: 'trigger-b' }, claimId: { value: 'claim-c' }, message: message() },
  ],
  fieldSelections: [
    {
      connectionId: { value: 'selection-a' },
      claimId: { value: 'claim-a' },
      binding: {
        owner: { kind: 'ENTITY_KIND_SPECIFICATION', id: id('events') },
        ownerUid: 'spec-uid',
        id: { value: 'binding-a' },
      },
      selectors: [{ path: { segments: ['customer', 'name'] } }, { opaque: { syntax: 'json-pointer', value: '/free' } }],
    },
  ],
});
const wire = (kind: 'component' | 'externalSystem' | 'domain', slug: string, uid: string) => ({
  [kind]: { id: id(slug), title: slug, messageClaims: claims() },
  system: { uid },
});
function fixture() {
  return buildModel([
    wire('component', 'checkout', 'checkout-uid'),
    wire('externalSystem', 'warehouse', 'warehouse-uid'),
    wire('domain', 'commerce', 'commerce-uid'),
    {
      specification: {
        id: id('events'),
        title: 'Events',
        messages: [
          {
            memberId: { value: 'message-a' },
            name: { value: 'Order placed' },
            schemas: [{ connectionId: { value: 'binding-a' }, role: 'ROLE_PAYLOAD' }],
          },
          { memberId: { value: 'message-b' }, name: { value: 'Order shipped' } },
        ],
      },
      system: { uid: 'spec-uid' },
    },
    { channel: { id: id('orders'), title: 'Orders' }, system: { uid: 'channel-uid' } },
  ]);
}
const of = (model: ReturnType<typeof fixture>, kind: Entity['kind']) =>
  model.entities.find((entity) => entity.kind === kind)!;
const leafIds = (items: MessageClaimLeaf[]) => items.map((item) => item.connectionId);
afterEach(cleanup);

it.each(['component', 'externalSystem', 'domain'] as const)(
  'projects independently authored %s claims with each leaf identity and no inference',
  (kind) => {
    const model = fixture();
    const owner = model.entities.find((entity) => entity.kind === kind)!;
    const select = vi.fn();
    const { container } = render(<MessageClaimsDetails entity={owner} model={model} onSelect={select} />);
    expect(screen.getByRole('heading', { name: 'Message claims' })).toBeDefined();
    for (const value of [
      'claim-a',
      'claim-b',
      'claim-c',
      'route-a',
      'route-b',
      'trigger-a',
      'trigger-b',
      'selection-a',
    ])
      expect(screen.getAllByText(value).length).toBeGreaterThan(0);
    expect(screen.getAllByText('SEND relative to the declaration owner')).toHaveLength(2);
    expect(screen.getByText('RECEIVE relative to the declaration owner')).toBeDefined();
    expect(container.querySelector('script')).toBeNull();
    expect(screen.getByText('PUSH_PULL')).toBeDefined();
    expect(screen.getAllByText('Undeclared').length).toBeGreaterThan(0);
    expect(screen.getByText('tenant: ""')).toBeDefined();
    expect(screen.getByText('region: "eu"')).toBeDefined();
    expect(screen.getByText('["customer","name"]')).toBeDefined();
    expect(screen.getByText('json-pointer: /free')).toBeDefined();
    expect(screen.getAllByText(/Opaque selector, unverified/)).toHaveLength(1);
    expect(screen.getByText(/Path as authored; Studio does not verify it/)).toBeDefined();
    expect(screen.getByText('No condition documented; delivery behavior is unknown.')).toBeDefined();
    expect(screen.getAllByText(/not evidence of execution, delivery or member rollup/)).toHaveLength(1);
    fireEvent.click(screen.getAllByRole('button', { name: 'shop/orders@18446744073709551615' })[0]);
    expect(select).toHaveBeenCalledWith(entityKey('channel', id('orders')));
    expect(model.slices).toHaveLength(0);
  },
);

it('keeps equal sibling claims, routes and triggers as separate ordered leaves', () => {
  const model = fixture();
  const owner = of(model, 'component');
  expect(leafIds(messageClaimLeaves(owner))).toEqual([
    'claim-a',
    'claim-b',
    'claim-c',
    'route-a',
    'route-b',
    'trigger-a',
    'trigger-b',
    'selection-a',
  ]);
  expect(messageClaimLeaves(owner).map((leaf) => leaf.path)).toContain('messageClaims.routes[1]');
});

it('resolves exact leaf subjects from partial raw subjects without substituting equal siblings', () => {
  const model = fixture();
  const owner = of(model, 'domain');
  const subjects = responsibilitySubjects(owner)
    .slice(1)
    .map((item) => item.subject);
  expect(subjects).toHaveLength(8);
  const route = subjects[4];
  const catalog = buildModel([
    {
      decision: {
        id: id('note'),
        subjects: [
          { subject: { connection: {} } },
          { subject: { connection: { ownerUid: 'commerce-uid' } } },
          { subject: { connection: { ownerUid: 'commerce-uid', owner: { kind: 'ENTITY_KIND_DOMAIN' } } } },
          { subject: route },
        ],
      },
    },
  ]).entities[0];
  render(<MessageClaimsDetails entity={catalog} model={model} onSelect={vi.fn()} />);
  expect(screen.getAllByText('route-b').length).toBeGreaterThan(0);
  expect(screen.queryByText('route-a')).toBeNull();
  (owner.raw.messageClaims as { routes: unknown[] }).routes.reverse();
  expect(resolveMessageClaimSubject(model, route)?.path).toBe('messageClaims.routes[0]');
  owner.uid = 'replacement';
  expect(resolveMessageClaimSubject(model, route)).toBeUndefined();
});

it('requires the complete identity inventory across peers, communications and every claim list', () => {
  const model = fixture();
  const owner = of(model, 'component');
  const subject = responsibilitySubjects(owner).find((item) => item.label.endsWith(': route-a'))!.subject;
  expect(resolveMessageClaimSubject(model, subject)?.connectionId).toBe('route-a');
  owner.raw.communications = [{ connectionId: { value: 'route-a' } }];
  expect(resolveMessageClaimSubject(model, subject)).toBeUndefined();
  const communication = { connection: { ...(subject as { connection: object }).connection, id: { value: 'route-a' } } };
  expect(resolveCommunicationSubject(model, communication)).toBeUndefined();
  owner.raw.communications = [];
  (owner.raw.messageClaims as { triggers: { connectionId: unknown }[] }).triggers[0].connectionId = {
    value: 'claim-a',
  };
  expect(resolveMessageClaimSubject(model, subject)).toBeUndefined();
});

it('lists loaded incoming claim routes on a Channel with a declared scope and no equal-sibling collapse', () => {
  const model = fixture();
  const channel = of(model, 'channel');
  const incoming = incomingMessageClaims(model, channel);
  expect(incoming.map((leaf) => [leaf.owner.id.slug, leaf.connectionId])).toEqual([
    ['checkout', 'route-a'],
    ['checkout', 'route-b'],
    ['warehouse', 'route-a'],
    ['warehouse', 'route-b'],
    ['commerce', 'route-a'],
    ['commerce', 'route-b'],
  ]);
  render(<MessageClaimsDetails entity={channel} model={model} onSelect={vi.fn()} />);
  expect(screen.getByText(/Incoming discovery is limited to loaded owners/)).toBeDefined();
  expect(screen.getAllByText('route-a')).toHaveLength(3);
  expect(screen.getAllByText('route-b')).toHaveLength(3);
});

it('lists each loaded incoming claim, trigger and field selection of a Specification as its own leaf', () => {
  const model = fixture();
  const specification = of(model, 'specification');
  const incoming = incomingMessageClaims(model, specification);
  expect(incoming.filter((leaf) => leaf.owner.kind === 'domain').map((leaf) => leaf.connectionId)).toEqual([
    'claim-a',
    'claim-b',
    'claim-c',
    'trigger-a',
    'trigger-b',
    'selection-a',
  ]);
  const unloaded = buildModel([wire('domain', 'commerce', 'commerce-uid')]);
  expect(incomingMessageClaims(unloaded, specification)).toHaveLength(6);
  expect(incomingMessageClaims(unloaded, of(model, 'channel'))).toHaveLength(2);
  render(<MessageClaimsDetails entity={specification} model={model} onSelect={vi.fn()} />);
  expect(screen.queryByText(/rollup of/)).toBeNull();
});

it('keeps authored selectors visible when the exact message, binding or Specification incarnation is unavailable', () => {
  const model = fixture();
  const owner = of(model, 'component');
  of(model, 'specification').uid = 'replacement-specification';
  render(<MessageClaimsDetails entity={owner} model={model} onSelect={vi.fn()} />);
  expect(screen.getAllByText('spec-uid').length).toBeGreaterThan(0);
  expect(screen.getAllByText('Unavailable exact message member').length).toBeGreaterThan(0);
  expect(screen.getAllByText('Unavailable exact PAYLOAD binding').length).toBeGreaterThan(0);
  expect(screen.getAllByText('message-a').length).toBeGreaterThan(0);
  expect(screen.getAllByText('binding-a').length).toBeGreaterThan(0);
});

it('resolves the exact message and PAYLOAD binding only against the matching loaded incarnation', () => {
  const model = fixture();
  render(<MessageClaimsDetails entity={of(model, 'component')} model={model} onSelect={vi.fn()} />);
  expect(screen.getAllByText('Order placed').length).toBeGreaterThan(0);
  expect(screen.getByText('PAYLOAD binding available in the loaded model')).toBeDefined();
});

it('does not crash on malformed or partial raw claim content', () => {
  const model = fixture();
  const owner = of(model, 'component');
  owner.raw.messageClaims = {
    claims: [null, 'text', {}, { connectionId: 7, message: [] }],
    routes: 7,
    triggers: {},
    fieldSelections: [{ selectors: [null, {}, { path: 'a.b' }, { opaque: { value: 3 } }], binding: 'x' }],
  };
  const { container } = render(<MessageClaimsDetails entity={owner} model={model} onSelect={vi.fn()} />);
  expect(container.textContent).toContain('Message claims');
  expect(messageClaimLeaves(owner)).toHaveLength(5);
  owner.raw.messageClaims = 'bad';
  expect(messageClaimLeaves(owner)).toEqual([]);
  expect(resolveMessageClaimSubject(model, { connection: { owner: 3, ownerUid: [], id: null } })).toBeUndefined();
  expect(resolveMessageClaimSubject(model, null)).toBeUndefined();
});

it('shows no claim surface for unrelated kinds and does not mutate the model', () => {
  const model = fixture();
  const before = JSON.stringify(model.entities.map((entity) => entity.raw));
  const { container } = render(
    <MessageClaimsDetails entity={of(model, 'specification')} model={model} onSelect={vi.fn()} />,
  );
  expect(container.textContent).toContain('Incoming discovery is limited to loaded owners');
  cleanup();
  const note = buildModel([{ team: { id: id('team') } }]).entities[0];
  expect(render(<MessageClaimsDetails entity={note} model={model} onSelect={vi.fn()} />).container.innerHTML).toBe('');
  expect(JSON.stringify(model.entities.map((entity) => entity.raw))).toBe(before);
});

it.each(['decision', 'document'])(
  'keeps the %s Inspector available with a partial and an exact claim subject',
  (kind) => {
    const model = fixture();
    const subject = responsibilitySubjects(of(model, 'component')).find((item) =>
      item.label.endsWith(': claim-b'),
    )!.subject;
    const catalog = buildModel([
      {
        [kind]: {
          id: id('note'),
          subjects: [{ subject: { connection: { ownerUid: 'checkout-uid', id: { value: 'claim-a' } } } }, { subject }],
        },
      },
    ]).entities[0];
    render(<Inspector entity={catalog} model={model} onSelect={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getByRole('heading', { name: 'Message claims' })).toBeDefined();
    expect(screen.getAllByText('claim-b').length).toBeGreaterThan(0);
    expect(screen.queryByText('claim-a')).toBeNull();
  },
);

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { buildModel, entityKey } from '@/lib/model';
import { communications, resolveCommunicationSubject, resolvedCommunicationMessage } from '@/lib/communication-intent';
import { responsibilitySubjects } from '@/lib/responsibility';
import { CommunicationIntentDetails } from './CommunicationIntentDetails';
import { Inspector } from './Inspector';

vi.mock('@/lib/useEntityHomes', () => ({
  useEntityHomes: () => ({ homes: new Map(), error: undefined, retry: vi.fn() }),
}));
vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: () => <button type="button">Copy context</button>,
}));

const id = (slug: string, version = '9007199254740993') => ({ namespace: 'shop', slug, version });
const declaration = (value: string, direction = 'COMMUNICATION_DIRECTION_SEND', member = 'message-a') => ({
  connectionId: { value },
  direction,
  peer: { externalSystem: { id: id('provider') } },
  message: {
    specification: { id: id('notifications') },
    ownerUid: 'spec-uid',
    kind: 'CONTRACT_MEMBER_KIND_MESSAGE',
    memberId: { value: member },
  },
  label: { value: 'Order hint' },
  condition: { value: '<script>inert()</script>\nwhen approved' },
});
function fixture() {
  return buildModel([
    {
      component: {
        id: id('checkout'),
        title: 'Checkout',
        communications: [
          declaration('send-a'),
          declaration('send-b'),
          declaration('receive', 'COMMUNICATION_DIRECTION_RECEIVE', 'message-b'),
        ],
      },
      system: { uid: 'owner-uid' },
    },
    { externalSystem: { id: id('provider'), title: 'Provider' } },
    {
      specification: {
        id: id('notifications'),
        title: 'Messages',
        messages: [
          { memberId: { value: 'message-a' }, name: { value: 'Hint A' } },
          { memberId: { value: 'message-b' }, name: { value: 'Hint B' } },
        ],
      },
      system: { uid: 'spec-uid' },
    },
  ]);
}
afterEach(cleanup);
it('preserves owner-local directions, equal siblings, inert conditions and loaded-only reference provenance', () => {
  const model = fixture();
  const select = vi.fn();
  const { container, rerender } = render(
    <CommunicationIntentDetails entity={model.entities[0]} model={model} onSelect={select} />,
  );
  expect(screen.getAllByText('Order hint')).toHaveLength(3);
  expect(screen.getAllByText('SEND relative to the declaration owner')).toHaveLength(2);
  expect(screen.getByText('RECEIVE relative to the declaration owner')).toBeDefined();
  expect(container.querySelector('script')).toBeNull();
  fireEvent.click(screen.getAllByRole('button', { name: 'shop/notifications@9007199254740993' })[0]);
  expect(select).toHaveBeenCalledWith(entityKey('specification', id('notifications')));
  rerender(<CommunicationIntentDetails entity={model.entities[1]} model={model} onSelect={select} />);
  expect(screen.getAllByText('Order hint')).toHaveLength(3);
  expect(screen.getByText(/SEND and RECEIVE do not reverse/)).toBeDefined();
  expect(model.slices).toHaveLength(0);
});
it.each(['decision', 'document'])(
  'inspects exact selected communications from %s despite partial raw subjects',
  (kind) => {
    const model = fixture();
    const owner = model.entities[0];
    const subject = responsibilitySubjects(owner)[1].subject;
    const catalog = buildModel([
      {
        [kind]: {
          id: id('note'),
          subjects: [
            { subject: { connection: {} } },
            { subject: { connection: { ownerUid: 'owner-uid' } } },
            { subject },
          ],
        },
      },
    ]).entities[0];
    render(<CommunicationIntentDetails entity={catalog} model={model} onSelect={vi.fn()} />);
    expect(screen.getByText('send-a')).toBeDefined();
    expect(screen.queryByText('send-b')).toBeNull();
    (owner.raw.communications as unknown[]).reverse();
    expect(resolveCommunicationSubject(model, subject)?.path).toBe('communications[2]');
    owner.uid = 'replacement';
    expect(resolveCommunicationSubject(model, subject)).toBeUndefined();
  },
);
it('requires the full identity inventory and exact message UID and member without fallback', () => {
  const model = fixture();
  const owner = model.entities[0];
  const subject = responsibilitySubjects(owner)[1].subject;
  expect(resolvedCommunicationMessage(model, communications(owner)[0])?.name).toBe('Hint A');
  model.entities[2].uid = 'replacement';
  expect(resolvedCommunicationMessage(model, communications(owner)[0])).toBeUndefined();
  model.entities[2].uid = 'spec-uid';
  (owner.raw.communications as ReturnType<typeof declaration>[])[0].message.memberId.value = 'removed';
  expect(resolvedCommunicationMessage(model, communications(owner)[0])).toBeUndefined();
  owner.raw.relationships = [{ connectionId: { value: 'send-a' } }];
  expect(resolveCommunicationSubject(model, subject)).toBeUndefined();
});
it('distinguishes undocumented conditions from present condition prose', () => {
  const model = fixture();
  const owner = model.entities[0];
  delete (owner.raw.communications as Partial<ReturnType<typeof declaration>>[])[0].condition;
  render(<CommunicationIntentDetails entity={owner} model={model} onSelect={vi.fn()} />);
  expect(screen.getByText('No condition documented; delivery behavior is unknown.')).toBeDefined();
});

it.each(['missing', 'recreated', 'unloaded'])(
  'retains authored exact Specification selection when message owner is %s',
  (state) => {
    const model = fixture();
    const owner = model.entities[0];
    const loaded = model.entities[2];
    if (state === 'missing') (loaded.raw.messages as unknown[]) = [];
    if (state === 'recreated') loaded.uid = 'replacement-specification';
    if (state === 'unloaded') {
      model.entities.splice(2, 1);
      model.byKey.delete(loaded.key);
    }
    render(<CommunicationIntentDetails entity={owner} model={model} onSelect={vi.fn()} />);
    expect(screen.getAllByText(/shop\/notifications@9007199254740993/)).toHaveLength(3);
    expect(screen.getAllByText('spec-uid')).toHaveLength(3);
    expect(screen.getAllByText('message-a')).toHaveLength(2);
    expect(screen.getAllByText('Unavailable exact message member')).toHaveLength(3);
  },
);

it.each(['decision', 'document'])(
  'keeps combined %s Inspector available with a partial connection and an exact communication',
  (kind) => {
    const model = fixture();
    const subject = responsibilitySubjects(model.entities[0])[1].subject;
    const catalog = buildModel([
      {
        [kind]: {
          id: id('note'),
          subjects: [{ subject: { connection: { ownerUid: 'owner-uid', id: { value: 'send-a' } } } }, { subject }],
        },
      },
    ]).entities[0];
    render(<Inspector entity={catalog} model={model} onSelect={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getByRole('heading', { name: 'Communication intent' })).toBeDefined();
    expect(screen.getAllByText('send-a').length).toBeGreaterThan(0);
  },
);

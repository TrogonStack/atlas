import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { buildModel, entityKey, wireKind } from '@/lib/model';
import { responsibilitySubjects } from '@/lib/responsibility';
import { SystemGroupDetails } from './SystemGroupDetails';

const id = (slug: string, version = '1') => ({ namespace: 'groups', slug, version });
const connection = (value: string) => ({ connectionId: { value } });

function fixture() {
  return buildModel([
    {
      system: { uid: 'platform-incarnation' },
      systemGroup: {
        id: id('platform'),
        title: 'Platform',
        scope: 'SYSTEM_GROUP_SCOPE_INTERNAL',
        members: [
          { ...connection('member-a'), member: { component: { id: id('api') } }, doc: { value: '<b>inert</b>' } },
          { ...connection('member-b'), member: { component: { id: id('api') } } },
          { ...connection('member-c'), member: { designGraph: { id: id('graph') } } },
          { ...connection('member-d'), member: { systemGroup: { id: id('platform') } } },
        ],
        relationships: [{ ...connection('relationship-a'), target: { id: id('billing') } }],
        actors: [
          {
            ...connection('actor-a'),
            target: { persona: { id: id('operator') } },
            direction: 'SYSTEM_ACTOR_DIRECTION_INBOUND',
            label: { value: 'operates' },
          },
        ],
      },
    },
    { systemGroup: { id: id('billing'), title: 'Billing' } },
    { component: { id: id('api'), title: 'API' } },
    {
      domain: {
        id: id('commerce'),
        title: 'Commerce',
        members: [{ ...connection('domain-a'), member: { systemGroup: { id: id('billing') } } }],
      },
    },
    { domain: { id: id('empty'), title: 'Empty' } },
  ]);
}

afterEach(cleanup);

it('maps the wire kind to the Studio kind', () => {
  expect(wireKind('ENTITY_KIND_SYSTEM_GROUP')).toBe('systemGroup');
  expect(fixture().byKind.get('systemGroup')).toHaveLength(2);
});

it('keeps equal members distinct, links exact targets and leaves the pending design graph kind unresolved', () => {
  const model = fixture();
  const select = vi.fn();
  const { container } = render(<SystemGroupDetails entity={model.entities[0]} model={model} onSelect={select} />);
  ['member-a', 'member-b', 'member-c', 'member-d', 'relationship-a', 'actor-a'].forEach((value) =>
    expect(screen.getByText(value)).toBeDefined(),
  );
  expect(screen.getAllByRole('button', { name: 'groups/api@1' })).toHaveLength(2);
  fireEvent.click(screen.getAllByRole('button', { name: 'groups/api@1' })[0]);
  expect(select).toHaveBeenCalledWith(entityKey('component', id('api')));
  expect(screen.getByText('Unavailable typed target (designGraph)')).toBeDefined();
  expect(screen.getByText('groups/operator@1 (not loaded)')).toBeDefined();
  expect(screen.getByText('inbound')).toBeDefined();
  expect(screen.getByText('operates')).toBeDefined();
  expect(screen.getByText('platform-incarnation')).toBeDefined();
  expect(container.querySelector('b')).toBeNull();
});

it('shows Domain members only when the Domain authors them', () => {
  const model = fixture();
  const commerce = model.entities.find((entity) => entity.id.slug === 'commerce')!;
  const empty = model.entities.find((entity) => entity.id.slug === 'empty')!;
  const { container, rerender } = render(<SystemGroupDetails entity={commerce} model={model} onSelect={vi.fn()} />);
  expect(screen.getByText('domain-a')).toBeDefined();
  expect(screen.getByRole('button', { name: 'groups/billing@1' })).toBeDefined();
  rerender(<SystemGroupDetails entity={empty} model={model} onSelect={vi.fn()} />);
  expect(container.textContent).toBe('');
});

it('offers every authored connection as an exact responsibility subject', () => {
  const model = fixture();
  const labels = responsibilitySubjects(model.entities[0]).map((item) => item.label);
  expect(labels).toEqual(
    expect.arrayContaining([
      'members[0]: member-a',
      'members[1]: member-b',
      'relationships[0]: relationship-a',
      'actors[0]: actor-a',
    ]),
  );
});

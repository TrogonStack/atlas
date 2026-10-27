import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { buildModel, entityKey } from '@/lib/model';
import { architecturalDeclarations, resolveArchitecturalSubject } from '@/lib/architectural-relationships';
import { responsibilitySubjects } from '@/lib/responsibility';
import { ArchitecturalRelationshipDetails } from './ArchitecturalRelationshipDetails';

const id = (slug: string, version = '18446744073709551615') => ({ namespace: 'shop', slug, version });
const relationship = (value: string) => ({
  connectionId: { value },
  target: { externalSystem: { id: id('provider') } },
  label: { value: 'submits orders' },
  doc: { value: '<script>inert()</script>\nCafe\u0301' },
});
function fixture() {
  return buildModel([
    {
      component: {
        id: id('checkout'),
        title: 'Checkout',
        relationships: ['out-a', 'out-b'].map(relationship),
        members: [
          { kind: 'ENTITY_KIND_EXTERNAL_SYSTEM', id: id('provider'), connectionId: { value: 'unsupported-member' } },
        ],
      },
      system: { uid: 'checkout-incarnation' },
    },
    { externalSystem: { id: id('provider'), title: 'Provider' }, system: { uid: 'provider-incarnation' } },
    { externalSystem: { id: id('provider', '1'), title: 'Earlier provider' } },
  ]);
}
afterEach(cleanup);
it('preserves parallel declarations, exact endpoint links, inert wording and separate incoming provenance', () => {
  const model = fixture();
  const owner = model.entities[0];
  const select = vi.fn();
  const { container, rerender } = render(
    <ArchitecturalRelationshipDetails entity={owner} model={model} onSelect={select} />,
  );
  expect(screen.getAllByText('submits orders')).toHaveLength(2);
  expect(screen.getByText('out-a')).toBeDefined();
  expect(screen.getByText('out-b')).toBeDefined();
  expect(screen.getAllByText('checkout-incarnation')).toHaveLength(2);
  expect(container.querySelector('script')).toBeNull();
  fireEvent.click(screen.getAllByRole('button', { name: 'shop/provider@18446744073709551615' })[0]);
  expect(select).toHaveBeenCalledWith(entityKey('externalSystem', id('provider')));
  expect(model.slices).toHaveLength(0);
  rerender(<ArchitecturalRelationshipDetails entity={model.entities[1]} model={model} onSelect={select} />);
  expect(screen.getAllByText('checkout-incarnation')).toHaveLength(2);
  expect(screen.getByText(/Incoming discovery is limited/)).toBeDefined();
  rerender(<ArchitecturalRelationshipDetails entity={model.entities[2]} model={model} onSelect={select} />);
  expect(screen.queryByText('submits orders')).toBeNull();
});
it('retains full connection subjects across reorder and refuses stale incarnation or incomplete inventory hydration', () => {
  const model = fixture();
  const owner = model.entities[0];
  const subjects = responsibilitySubjects(owner)
    .slice(1)
    .map((item) => item.subject);
  expect(subjects).toHaveLength(2);
  const selected = subjects[0];
  expect(selected).toEqual({
    connection: {
      owner: { kind: 'ENTITY_KIND_COMPONENT', id: id('checkout') },
      ownerUid: 'checkout-incarnation',
      id: { value: 'out-a' },
    },
  });
  (owner.raw.relationships as unknown[]).reverse();
  expect(resolveArchitecturalSubject(model, selected)?.path).toBe('relationships[1]');
  expect(
    responsibilitySubjects(owner)
      .slice(1)
      .map((item) => item.subject),
  ).toEqual([...subjects].reverse());
  owner.uid = 'replacement-incarnation';
  expect(resolveArchitecturalSubject(model, selected)).toBeUndefined();
  owner.uid = 'checkout-incarnation';
  owner.raw.relationships = [relationship('out-a'), relationship('out-a')];
  expect(resolveArchitecturalSubject(model, selected)).toBeUndefined();
});
it('hydrates authored catalog subjects by exact UID and ID without attaching equal siblings', () => {
  const model = fixture();
  const subject = responsibilitySubjects(model.entities[0])[1].subject;
  const catalog = buildModel([{ document: { id: id('note'), subjects: [{ subject }] } }]).entities[0];
  render(<ArchitecturalRelationshipDetails entity={catalog} model={model} onSelect={vi.fn()} />);
  expect(screen.getByText('out-a')).toBeDefined();
  expect(screen.queryByText('out-b')).toBeNull();
  expect(architecturalDeclarations(catalog)).toEqual([]);
});
it.each(['document', 'decision'])(
  'keeps %s inspection available for a raw connection subject without an ID',
  (kind) => {
    const model = fixture();
    const subject = responsibilitySubjects(model.entities[0])[1].subject;
    const catalog = buildModel([
      {
        [kind]: {
          id: id('note'),
          subjects: [
            {
              subject: {
                connection: {
                  owner: { kind: 'ENTITY_KIND_COMPONENT', id: id('checkout') },
                  ownerUid: 'checkout-incarnation',
                },
              },
            },
            { subject },
          ],
        },
      },
    ]).entities[0];
    render(<ArchitecturalRelationshipDetails entity={catalog} model={model} onSelect={vi.fn()} />);
    expect(screen.getByText('out-a')).toBeDefined();
    expect(screen.queryByText('out-b')).toBeNull();
  },
);

it.each(['document', 'decision'])('keeps %s inspection available for a raw connection without its owner', (kind) => {
  const model = fixture();
  const subject = responsibilitySubjects(model.entities[0])[1].subject;
  const catalog = buildModel([
    {
      [kind]: {
        id: id('note'),
        subjects: [
          { subject: { connection: { ownerUid: 'checkout-incarnation', id: { value: 'out-a' } } } },
          { subject },
        ],
      },
    },
  ]).entities[0];
  render(<ArchitecturalRelationshipDetails entity={catalog} model={model} onSelect={vi.fn()} />);
  expect(screen.getByText('out-a')).toBeDefined();
  expect(screen.queryByText('out-b')).toBeNull();
});

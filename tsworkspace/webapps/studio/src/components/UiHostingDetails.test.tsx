import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { buildModel } from '@/lib/model';
import { hasUnambiguousOwnerIdentities, responsibilitySubjects } from '@/lib/responsibility';
import { Inspector } from './Inspector';

vi.mock('@/lib/useEntityHomes', () => ({
  useEntityHomes: () => ({ homes: new Map(), error: undefined, retry: vi.fn() }),
}));
vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: () => <button type="button">Copy context</button>,
}));

const version = '9007199254740993';
const id = (slug: string) => ({ namespace: 'shop', slug, version });
const hosting = (value: string, doc?: string) => ({
  connectionId: { value },
  ui: { id: id('review') },
  label: { value: 'Review portal 🦜' },
  ...(doc === undefined ? {} : { doc: { value: doc } }),
});
function fixture() {
  return buildModel([
    {
      component: {
        id: id('portal'),
        title: 'Portal',
        uiHostings: [hosting('hosting-a'), hosting('hosting-b', '<script>inert()</script>\nwhy')],
      },
      system: { uid: 'component-uid' },
    },
    { ui: { id: id('review'), title: 'Review' }, system: { uid: 'ui-uid' } },
  ]);
}
afterEach(cleanup);

it('shows native hosting occurrence identities and inert authored wording without inferring interactions', () => {
  const model = fixture();
  render(<Inspector model={model} entity={model.entities[0]} onClose={vi.fn()} onSelect={vi.fn()} />);
  expect(screen.getByRole('heading', { name: 'UI hosting and human interaction' })).toBeTruthy();
  expect(screen.getAllByText('component-uid').length).toBeGreaterThanOrEqual(2);
  expect(screen.getByText('hosting-a')).toBeTruthy();
  expect(screen.getByText('hosting-b')).toBeTruthy();
  expect(screen.getAllByText('Review portal 🦜')).toHaveLength(2);
  expect(screen.getByText('<script>inert()</script> why')).toBeTruthy();
  expect(document.querySelector('script')).toBeNull();
  expect(screen.getByText(/Select an exact hosting and a complete slice tuple/)).toBeTruthy();
});

it('includes hostings in the complete owner identity inventory and exact subject list', () => {
  const model = fixture();
  const owner = model.entities[0];
  expect(
    responsibilitySubjects(owner).flatMap((item) =>
      'connection' in item.subject ? [item.subject.connection.id.value] : [],
    ),
  ).toEqual(['hosting-a', 'hosting-b']);
  expect(hasUnambiguousOwnerIdentities(owner)).toBe(true);
  owner.raw.relationships = [{ connectionId: { value: 'hosting-a' } }];
  expect(hasUnambiguousOwnerIdentities(owner)).toBe(false);
});

it('rejects incomplete hosting identities alongside existing roles', () => {
  const owner = fixture().entities[0];
  owner.raw.uiHostings = [{ ui: { id: id('review') }, label: { value: 'Missing identity' } }];
  expect(hasUnambiguousOwnerIdentities(owner)).toBe(false);
});

it('does not infer a hosting from a raw Component member', () => {
  const model = buildModel([
    {
      component: { id: id('portal'), title: 'Portal', members: [{ kind: 'ENTITY_KIND_UI', id: id('review') }] },
      system: { uid: 'component-uid' },
    },
  ]);
  render(<Inspector model={model} entity={model.entities[0]} onClose={vi.fn()} onSelect={vi.fn()} />);
  expect(screen.getByText('No UI hosting assertions in this loaded owner.')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Check selected interaction' })).toBeNull();
});

for (const kind of ['decision', 'document'] as const) {
  it(`shows an exact loaded hosting cited by a ${kind} with its containing owner's provenance`, () => {
    const model = fixture();
    const citation = buildModel([
      {
        [kind]: {
          id: id('citation'),
          title: 'Cited hosting',
          subjects: [
            {
              subject: {
                connection: {
                  owner: { kind: 'ENTITY_KIND_COMPONENT', id: id('portal') },
                  ownerUid: 'component-uid',
                  id: { value: 'hosting-b' },
                },
              },
            },
          ],
        },
        system: { uid: 'citation-uid' },
      },
    ]).entities[0];
    render(<Inspector model={model} entity={citation} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByRole('heading', { name: 'UI hosting and human interaction' })).toBeTruthy();
    expect(screen.getByText('Review portal 🦜')).toBeTruthy();
    expect(screen.getByText('component-uid')).toBeTruthy();
    expect(screen.getByText('hosting-b')).toBeTruthy();
    expect(screen.queryByText('hosting-a')).toBeNull();
    expect(screen.getByText('<script>inert()</script> why')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Check selected interaction' })).toBeNull();
  });
}

it('does not substitute a loaded hosting for a stale or incomplete cited selector', () => {
  const model = fixture();
  const connection = {
    owner: { kind: 'ENTITY_KIND_COMPONENT', id: id('portal') },
    ownerUid: 'component-uid',
    id: { value: 'hosting-a' },
  };
  for (const broken of [
    { ...connection, ownerUid: 'old-component-uid' },
    { ...connection, owner: { ...connection.owner, id: { ...id('portal'), version: '1' } } },
    { ...connection, id: { value: 'missing-hosting' } },
  ]) {
    const citation = buildModel([
      {
        document: { id: id('citation'), title: 'Cited hosting', subjects: [{ subject: { connection: broken } }] },
        system: { uid: 'citation-uid' },
      },
    ]).entities[0];
    const rendered = render(<Inspector model={model} entity={citation} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.queryByRole('heading', { name: 'UI hosting and human interaction' })).toBeNull();
    rendered.unmount();
  }
});

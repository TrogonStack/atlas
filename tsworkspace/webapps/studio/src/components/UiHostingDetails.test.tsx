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

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { buildModel, entityKey } from '@/lib/model';
import { UiHostingDetails } from './UiHostingDetails';

const inspect = vi.hoisted(() => vi.fn());
vi.mock('@/lib/api', () => ({ inspectHumanInteraction: inspect }));
const version = '9007199254740993';
const id = (slug: string) => ({ namespace: 'shop', slug, version });
const edge = (connectionId: string, field: string, slug: string) => ({
  connectionId: { value: connectionId },
  [field]: { id: id(slug) },
});
function fixture() {
  return buildModel([
    {
      component: {
        id: id('portal'),
        title: 'Portal',
        uiHostings: ['hosting-a', 'hosting-b'].map((value) => ({
          connectionId: { value },
          ui: { id: id('review') },
          label: { value: 'Review' },
        })),
      },
      system: { uid: 'component-uid' },
    },
    { ui: { id: id('review'), title: 'Review' }, system: { uid: 'ui-uid' } },
    {
      uiSlice: {
        id: id('display'),
        title: 'Show summary',
        persona: edge('persona-a', 'persona', 'reviewer'),
        ui: edge('ui-a', 'ui', 'review'),
        sourceReadModels: [
          edge('observation-a', 'readModel', 'summary'),
          edge('observation-b', 'readModel', 'summary'),
        ],
      },
      system: { uid: 'slice-uid' },
    },
    {
      commandSlice: {
        id: id('issue'),
        title: 'Approve',
        persona: edge('issue-persona', 'persona', 'reviewer'),
        ui: edge('issue-ui', 'ui', 'review'),
        command: edge('issue-command', 'command', 'approve'),
      },
      system: { uid: 'issuance-uid' },
    },
  ]);
}
const selected = (kind: string, slug: string, ownerUid: string, value: string) => ({
  owner: { kind, id: id(slug) },
  ownerUid,
  id: { value },
});
const displayRequest = {
  hosting: selected('ENTITY_KIND_COMPONENT', 'portal', 'component-uid', 'hosting-b'),
  display: {
    persona: selected('ENTITY_KIND_UI_SLICE', 'display', 'slice-uid', 'persona-a'),
    ui: selected('ENTITY_KIND_UI_SLICE', 'display', 'slice-uid', 'ui-a'),
    sourceReadModel: selected('ENTITY_KIND_UI_SLICE', 'display', 'slice-uid', 'observation-b'),
  },
};
function chooseDisplay() {
  fireEvent.change(screen.getByLabelText('Exact hosting occurrence'), { target: { value: 'hosting-b' } });
  fireEvent.change(screen.getByLabelText('Human path type'), { target: { value: 'display' } });
  fireEvent.change(screen.getByLabelText('Exact slice owner'), {
    target: { value: entityKey('uiSlice', id('display')) },
  });
  fireEvent.change(screen.getByLabelText('Persona occurrence'), { target: { value: 'persona-a' } });
  fireEvent.change(screen.getByLabelText('UI occurrence'), { target: { value: 'ui-a' } });
  fireEvent.change(screen.getByLabelText('ReadModel observation occurrence'), { target: { value: 'observation-b' } });
}
const response = (request: unknown) => ({
  request,
  state: 'CATALOG_RESOLUTION_STATE_RESOLVED',
  consistency: 'HUMAN_INTERACTION_CONSISTENCY_PER_OWNER_CHECKED_READ_SET',
  scope: {
    branch: 'review',
    revision: 'checked-read-set',
    authorizedNamespaces: ['shop'],
    visited: [
      {
        owner: { kind: 'ENTITY_KIND_COMPONENT', id: id('portal') },
        uid: 'checked-owner-uid',
        etag: 'checked-owner-etag',
      },
    ],
  },
  edges: [
    {
      role: 'HUMAN_INTERACTION_ROLE_READ_MODEL',
      selection: displayRequest.display.sourceReadModel,
      field: 'source_read_models[1]',
      target: { kind: 'ENTITY_KIND_READ_MODEL', id: id('summary') },
      state: 'CATALOG_RESOLUTION_STATE_RESOLVED',
      checkedTarget: {
        owner: { kind: 'ENTITY_KIND_READ_MODEL', id: id('summary') },
        uid: 'checked-target-uid',
        etag: 'checked-target-etag',
      },
    },
  ],
});
afterEach(() => {
  cleanup();
  inspect.mockReset();
  window.history.replaceState({}, '', '/');
});

it('requires explicit full tuple choices and checks only the selected equal-target observation', async () => {
  const model = fixture();
  inspect.mockImplementation(async (request) => response(request));
  window.history.replaceState({}, '', '/?branch=review');
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  expect((screen.getByRole('button', { name: 'Check selected interaction' }) as HTMLButtonElement).disabled).toBe(true);
  expect(inspect).not.toHaveBeenCalled();
  chooseDisplay();
  fireEvent.click(screen.getByRole('button', { name: 'Check selected interaction' }));
  await screen.findByText('Per-owner checked read set');
  expect(inspect).toHaveBeenCalledTimes(1);
  expect(inspect.mock.calls[0][0]).toEqual(displayRequest);
  expect(inspect.mock.calls[0][1]).toMatchObject({ branch: 'review' });
  expect(screen.getByText(/not an atomic snapshot/)).toBeTruthy();
  expect(screen.getByText(/checked-owner-etag/)).toBeTruthy();
  expect(screen.getByText(/checked-target-uid/)).toBeTruthy();
  expect(screen.getByText(/source_read_models\[1\]/)).toBeTruthy();
});

it('aborts and clears old receipts and pending responses when checked model content changes', async () => {
  const model = fixture();
  let finish: ((value: unknown) => void) | undefined;
  inspect.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const rendered = render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(screen.getByRole('button', { name: 'Check selected interaction' }));
  const signal = inspect.mock.calls[0][1].signal as AbortSignal;
  const newer = fixture();
  newer.entities[2].raw.title = 'Changed independently';
  rendered.rerender(<UiHostingDetails entity={newer.entities[0]} model={newer} onSelect={vi.fn()} />);
  expect(signal.aborted).toBe(true);
  await act(async () => finish?.(response(displayRequest)));
  expect(screen.queryByText('Per-owner checked read set')).toBeNull();
  expect(screen.queryByText(/checked-owner-etag/)).toBeNull();
  expect((screen.getByRole('button', { name: 'Check selected interaction' }) as HTMLButtonElement).disabled).toBe(true);
});

it('does not offer ambiguous owners or incomplete role tuples as a checked path', () => {
  const model = fixture();
  model.entities[2].raw.sourceReadModels = [edge('persona-a', 'readModel', 'summary')];
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('Human path type'), { target: { value: 'display' } });
  expect(screen.queryByRole('option', { name: /Show summary/ })).toBeNull();
  expect((screen.getByRole('button', { name: 'Check selected interaction' }) as HTMLButtonElement).disabled).toBe(true);
  expect(inspect).not.toHaveBeenCalled();
});

it('keeps unavailable receipts generic and does not replace them with loaded endpoint facts', async () => {
  const model = fixture();
  inspect.mockImplementation(async (request) => ({
    request,
    state: 'CATALOG_RESOLUTION_STATE_UNAVAILABLE',
    consistency: 'HUMAN_INTERACTION_CONSISTENCY_PER_OWNER_CHECKED_READ_SET',
    scope: { revision: 'unavailable-read-set' },
    edges: [],
  }));
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(screen.getByRole('button', { name: 'Check selected interaction' }));
  expect(await screen.findByText('Unavailable endpoint')).toBeTruthy();
  expect(screen.queryByText(/checked-target-uid/)).toBeNull();
});

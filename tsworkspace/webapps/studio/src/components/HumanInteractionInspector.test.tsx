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

it('cannot submit a checked tuple when the selected slice has an incomplete raw owner identity', () => {
  const model = fixture();
  model.entities[2].raw.id = { namespace: 'shop', slug: 'display' };
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  expect((screen.getByRole('button', { name: 'Check selected interaction' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Check selected interaction' }));
  expect(inspect).not.toHaveBeenCalled();
});

it('aborts old checks after same-reference model content changes without reusing stale receipts', async () => {
  const model = fixture();
  let finish: ((value: unknown) => void) | undefined;
  inspect.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const entity = model.entities[0];
  const rendered = render(<UiHostingDetails entity={entity} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(screen.getByRole('button', { name: 'Check selected interaction' }));
  expect(inspect).toHaveBeenCalledTimes(1);
  const signal = inspect.mock.calls[0][1].signal as AbortSignal;
  model.entities[2].raw.title = 'Changed in the same loaded model';
  rendered.rerender(<UiHostingDetails entity={entity} model={model} onSelect={vi.fn()} />);
  expect(signal.aborted).toBe(true);
  await act(async () => finish?.(response(displayRequest)));
  expect(screen.queryByText('Per-owner checked read set')).toBeNull();
  expect(screen.queryByText(/checked-owner-etag/)).toBeNull();
  expect((screen.getByRole('button', { name: 'Check selected interaction' }) as HTMLButtonElement).disabled).toBe(true);
});

// Receipt freshness, client half: a checked receipt, pending request or error is
// shown only while it still belongs to the current selection, branch and loaded model.
const checkButton = () => screen.getByRole('button', { name: 'Check selected interaction' }) as HTMLButtonElement;
const receiptShown = () => screen.queryByText('Per-owner checked read set') !== null;
function deferredBackend() {
  const calls: {
    request: unknown;
    signal: AbortSignal;
    resolve: (value: unknown) => void;
    reject: (reason: unknown) => void;
  }[] = [];
  inspect.mockImplementation(
    (request, options) =>
      new Promise((resolve, reject) => {
        calls.push({ request, signal: options.signal as AbortSignal, resolve, reject });
      }),
  );
  return calls;
}
const observationA = { target: { value: 'observation-a' } };
const displayRequestFor = (observation: string) => ({
  ...displayRequest,
  display: {
    ...displayRequest.display,
    sourceReadModel: selected('ENTITY_KIND_UI_SLICE', 'display', 'slice-uid', observation),
  },
});
async function checkedReceipt(model = fixture()) {
  inspect.mockImplementation(async (request) => response(request));
  const rendered = render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  await screen.findByText('Per-owner checked read set');
  return { model, rendered };
}

const selectionChanges: [string, string, string][] = [
  ['hosting occurrence', 'Exact hosting occurrence', 'hosting-a'],
  ['human path type', 'Human path type', 'issuance'],
  ['slice owner', 'Exact slice owner', ''],
  ['persona occurrence', 'Persona occurrence', ''],
  ['UI occurrence', 'UI occurrence', ''],
  ['endpoint occurrence', 'ReadModel observation occurrence', 'observation-a'],
];
for (const [name, label, value] of selectionChanges) {
  it(`drops a checked receipt and its tuple as soon as the ${name} selection changes`, async () => {
    await checkedReceipt();
    fireEvent.change(screen.getByLabelText(label), { target: { value } });
    expect(receiptShown()).toBe(false);
    expect(screen.queryByText(/checked-owner-etag/)).toBeNull();
    expect(screen.queryByText(/checked-target-uid/)).toBeNull();
    expect(screen.queryByText('Resolved checked path')).toBeNull();
    expect(checkButton().disabled).toBe(!['hosting occurrence', 'endpoint occurrence'].includes(name));
  });
}

it('does not reuse a receipt when the old selection is chosen again without a new check', async () => {
  await checkedReceipt();
  fireEvent.change(screen.getByLabelText('ReadModel observation occurrence'), observationA);
  fireEvent.change(screen.getByLabelText('ReadModel observation occurrence'), { target: { value: 'observation-b' } });
  expect(receiptShown()).toBe(false);
  expect(inspect).toHaveBeenCalledTimes(1);
});

it('ignores a slow response for selection A that arrives after selection B was checked and answered', async () => {
  const model = fixture();
  const calls = deferredBackend();
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  expect(calls).toHaveLength(1);
  fireEvent.change(screen.getByLabelText('ReadModel observation occurrence'), observationA);
  expect(calls[0].signal.aborted).toBe(true);
  fireEvent.click(checkButton());
  expect(calls).toHaveLength(2);
  expect(calls[0].request).toEqual(displayRequest);
  expect(calls[1].request).toEqual(displayRequestFor('observation-a'));
  expect(calls[1].signal.aborted).toBe(false);
  const bReceipt = response(calls[1].request);
  bReceipt.scope.visited[0].etag = 'etag-for-b';
  bReceipt.edges[0].selection = displayRequestFor('observation-a').display.sourceReadModel;
  await act(async () => calls[1].resolve(bReceipt));
  expect(await screen.findByText(/etag-for-b/)).toBeTruthy();
  const aReceipt = response(calls[0].request);
  aReceipt.scope.visited[0].etag = 'etag-for-a';
  await act(async () => calls[0].resolve(aReceipt));
  expect(screen.queryByText(/etag-for-a/)).toBeNull();
  expect(screen.getByText(/etag-for-b/)).toBeTruthy();
  const shown = screen.getByText('Per-owner checked read set').parentElement?.textContent ?? '';
  expect(shown).toContain('observation-a');
  expect(shown).not.toContain('observation-b');
  expect(checkButton().disabled).toBe(false);
});

it('ignores a slow response for selection A that arrives while selection B is still pending', async () => {
  const model = fixture();
  const calls = deferredBackend();
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  fireEvent.change(screen.getByLabelText('ReadModel observation occurrence'), observationA);
  fireEvent.click(checkButton());
  await act(async () => calls[0].resolve(response(calls[0].request)));
  expect(receiptShown()).toBe(false);
  expect(screen.queryByText('Resolved checked path')).toBeNull();
  expect(screen.getByText(/Checking selected backend context/)).toBeTruthy();
  expect(checkButton().disabled).toBe(true);
  await act(async () => calls[1].resolve(response(calls[1].request)));
  expect(receiptShown()).toBe(true);
  expect(screen.queryByText(/Checking selected backend context/)).toBeNull();
});

it('ignores a late response for A when selection moved on and nothing new was checked', async () => {
  const model = fixture();
  const calls = deferredBackend();
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  fireEvent.change(screen.getByLabelText('Exact hosting occurrence'), { target: { value: 'hosting-a' } });
  expect(screen.queryByText(/Checking selected backend context/)).toBeNull();
  await act(async () => calls[0].resolve(response(calls[0].request)));
  expect(receiptShown()).toBe(false);
  expect(screen.queryByText(/checked-owner-etag/)).toBeNull();
});

it('ignores a late failure for A after B has been checked and answered', async () => {
  const model = fixture();
  const calls = deferredBackend();
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  fireEvent.change(screen.getByLabelText('ReadModel observation occurrence'), observationA);
  fireEvent.click(checkButton());
  await act(async () => calls[1].resolve(response(calls[1].request)));
  await act(async () => calls[0].reject(new Error('late failure for A')));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(receiptShown()).toBe(true);
  expect(checkButton().disabled).toBe(false);
});

it('does not show an abort rejection from the superseded request as an error', async () => {
  const model = fixture();
  const calls = deferredBackend();
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  fireEvent.change(screen.getByLabelText('ReadModel observation occurrence'), observationA);
  await act(async () => calls[0].reject(new DOMException('aborted', 'AbortError')));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(receiptShown()).toBe(false);
  expect(checkButton().disabled).toBe(false);
});

it('resolves A and B in order without letting the first answer mask the pending second', async () => {
  const model = fixture();
  const calls = deferredBackend();
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  fireEvent.change(screen.getByLabelText('ReadModel observation occurrence'), observationA);
  fireEvent.click(checkButton());
  await act(async () => calls[0].resolve(response(calls[0].request)));
  await act(async () => calls[0].resolve(response(calls[0].request)));
  expect(receiptShown()).toBe(false);
  expect(checkButton().disabled).toBe(true);
});

it('keeps a pending request from showing a receipt after the component unmounts', async () => {
  const model = fixture();
  const calls = deferredBackend();
  const rendered = render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  rendered.unmount();
  expect(calls[0].signal.aborted).toBe(true);
  await act(async () => calls[0].resolve(response(calls[0].request)));
  expect(receiptShown()).toBe(false);
});

it('shows only the new failure after a receipt, never the superseded receipt beside the error', async () => {
  const model = fixture();
  let fail = false;
  inspect.mockImplementation(async (request) => {
    if (fail) throw new Error('backend unavailable');
    return response(request);
  });
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  await screen.findByText('Per-owner checked read set');
  fail = true;
  fireEvent.click(checkButton());
  expect((await screen.findByRole('alert')).textContent).toContain('Resolution remains unknown.');
  expect(receiptShown()).toBe(false);
  expect(screen.queryByText('Resolved checked path')).toBeNull();
  expect(screen.queryByText(/checked-owner-etag/)).toBeNull();
  expect(checkButton().disabled).toBe(false);
});

it('clears an error when the selection changes and never revives the receipt that preceded it', async () => {
  const model = fixture();
  inspect.mockRejectedValue(new Error('backend unavailable'));
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  await screen.findByRole('alert');
  fireEvent.change(screen.getByLabelText('ReadModel observation occurrence'), observationA);
  expect(screen.queryByRole('alert')).toBeNull();
  expect(receiptShown()).toBe(false);
  expect(screen.queryByText('Resolved checked path')).toBeNull();
});

it('stringifies a non-Error failure and keeps resolution unknown', async () => {
  const model = fixture();
  inspect.mockRejectedValue('gateway refused');
  render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  expect((await screen.findByRole('alert')).textContent).toBe(
    'Interaction inspection unavailable: gateway refused. Resolution remains unknown.',
  );
  expect(receiptShown()).toBe(false);
});

it('clears an error and its selection when the loaded model changes', async () => {
  const model = fixture();
  inspect.mockRejectedValue(new Error('backend unavailable'));
  const rendered = render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  await screen.findByRole('alert');
  const newer = fixture();
  newer.entities[2].raw.title = 'Changed independently';
  rendered.rerender(<UiHostingDetails entity={newer.entities[0]} model={newer} onSelect={vi.fn()} />);
  expect(screen.queryByRole('alert')).toBeNull();
  expect(checkButton().disabled).toBe(true);
});

it('aborts and drops a receipt when only the selected slice owner revision (uid) changes', async () => {
  const model = fixture();
  const calls = deferredBackend();
  const rendered = render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  await act(async () => calls[0].resolve(response(calls[0].request)));
  expect(receiptShown()).toBe(true);
  const recreated = fixture();
  recreated.entities[2].uid = 'slice-uid-recreated';
  rendered.rerender(<UiHostingDetails entity={recreated.entities[0]} model={recreated} onSelect={vi.fn()} />);
  expect(receiptShown()).toBe(false);
  expect(checkButton().disabled).toBe(true);
  expect(screen.getByLabelText<HTMLSelectElement>('Exact slice owner').value).toBe('');
});

it('aborts a pending check when the containing Component is recreated with equal content', async () => {
  const model = fixture();
  const calls = deferredBackend();
  const rendered = render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  const recreated = fixture();
  recreated.entities[0].uid = 'component-uid-recreated';
  rendered.rerender(<UiHostingDetails entity={recreated.entities[0]} model={recreated} onSelect={vi.fn()} />);
  expect(calls[0].signal.aborted).toBe(true);
  await act(async () => calls[0].resolve(response(calls[0].request)));
  expect(receiptShown()).toBe(false);
  expect(screen.getByLabelText<HTMLSelectElement>('Exact hosting occurrence').value).toBe('');
});

it('drops a receipt when the selected occurrence is removed from the loaded slice', async () => {
  const { model, rendered } = await checkedReceipt();
  const edited = fixture();
  edited.entities[2].raw.sourceReadModels = [edge('observation-a', 'readModel', 'summary')];
  rendered.rerender(<UiHostingDetails entity={edited.entities[0]} model={edited} onSelect={vi.fn()} />);
  expect(model).not.toBe(edited);
  expect(receiptShown()).toBe(false);
  expect(screen.queryByRole('option', { name: /observation-b/ })).toBeNull();
  expect(checkButton().disabled).toBe(true);
});

it('drops a receipt when the hosting set of the containing Component changes', async () => {
  const { rendered } = await checkedReceipt();
  const edited = fixture();
  (edited.entities[0].raw.uiHostings as unknown[]).pop();
  rendered.rerender(<UiHostingDetails entity={edited.entities[0]} model={edited} onSelect={vi.fn()} />);
  expect(receiptShown()).toBe(false);
  expect(screen.queryByRole('option', { name: /hosting-b/ })).toBeNull();
  expect(checkButton().disabled).toBe(true);
});

it('keeps a receipt across an unrelated rerender of the same loaded model', async () => {
  const { model, rendered } = await checkedReceipt();
  rendered.rerender(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  expect(receiptShown()).toBe(true);
  expect(inspect).toHaveBeenCalledTimes(1);
});

it('aborts, drops the receipt and the selection when the branch switches', async () => {
  window.history.replaceState({}, '', '/?branch=review');
  const model = fixture();
  const calls = deferredBackend();
  const rendered = render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  expect(inspect.mock.calls[0][1]).toMatchObject({ branch: 'review' });
  window.history.replaceState({}, '', '/?branch=other');
  rendered.rerender(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  expect(calls[0].signal.aborted).toBe(true);
  await act(async () => calls[0].resolve(response(calls[0].request)));
  expect(receiptShown()).toBe(false);
  expect(checkButton().disabled).toBe(true);
  chooseDisplay();
  fireEvent.click(checkButton());
  expect(inspect.mock.calls[1][1]).toMatchObject({ branch: 'other' });
});

it('drops a checked receipt when the branch switches back to the default view', async () => {
  window.history.replaceState({}, '', '/?branch=review');
  const { model, rendered } = await checkedReceipt();
  window.history.replaceState({}, '', '/');
  rendered.rerender(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  expect(receiptShown()).toBe(false);
  chooseDisplay();
  fireEvent.click(checkButton());
  await screen.findByText('Per-owner checked read set');
  expect(inspect.mock.calls[1][1].branch).toBeUndefined();
});

it('removes the check, aborts it and drops the receipt when a historical view is selected', async () => {
  const model = fixture();
  const calls = deferredBackend();
  const rendered = render(<UiHostingDetails entity={model.entities[0]} model={model} onSelect={vi.fn()} />);
  chooseDisplay();
  fireEvent.click(checkButton());
  rendered.rerender(
    <UiHostingDetails
      entity={model.entities[0]}
      model={model}
      onSelect={vi.fn()}
      asOf="a0b1c2d3-0000-4000-8000-000000000000"
    />,
  );
  expect(calls[0].signal.aborted).toBe(true);
  await act(async () => calls[0].resolve(response(calls[0].request)));
  expect(receiptShown()).toBe(false);
  expect(screen.queryByRole('button', { name: 'Check selected interaction' })).toBeNull();
  expect(screen.getByText(/Current interaction inspection is unavailable for historical views/)).toBeTruthy();
});

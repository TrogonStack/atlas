import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from '@/lib/api';
import type { CatalogInspection } from '@/lib/decision-document';
import { buildModel } from '@/lib/model';
import { DecisionDocumentInspector } from './DecisionDocumentInspector';

const id = { namespace: 'catalog', slug: 'note', version: '7' };
const subject = {
  port: {
    product: { id: { ...id, slug: 'product', version: '9' } },
    ownerUid: 'product-uid',
    direction: 'DATA_PORT_DIRECTION_OUTPUT',
    portId: { value: 'port-b' },
  },
};
const digest = { sha256: 'a'.repeat(64) };
const text = '# Note\n<img src="https://example.test/pixel" onerror="alert(1)">\n[go](javascript:alert(1))\n';
function entity(kind = 'document') {
  return buildModel([
    {
      [kind]: {
        id,
        title: { value: 'Retained title' },
        subjects: [{ subject, connectionId: { value: 'subject-b' } }],
        retained: { mediaType: { value: 'text/markdown' }, content: text, digest },
        anchors: [{ anchorId: { value: 'anchor-b' }, fragment: { value: 'Delivery' }, label: { value: 'Delivery' } }],
      },
      system: { uid: 'document-incarnation' },
    },
  ]).entities[0];
}
beforeEach(() => {
  for (const method of [
    'listDecisionAmendments',
    'getDecisionDetail',
    'getDocumentDetail',
    'listIncomingDecisionDeclarations',
    'listIncomingDocumentDeclarations',
    'listSubjectDecisions',
    'listSubjectDocuments',
  ] as const)
    vi.spyOn(api, method).mockResolvedValue({
      scope: { branch: 'review', revision: 'checked-revision' },
      incomplete: true,
      failures: [{ field: 'subjects', message: 'Unavailable target', code: 14 }],
    });
  window.history.replaceState({}, '', '/?branch=review');
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.history.replaceState({}, '', '/');
});
describe('native catalog inspector', () => {
  it('preserves pending and completed queries across same-context parent renders', async () => {
    let resolveDetail: ((value: CatalogInspection) => void) | undefined;
    vi.mocked(api.getDocumentDetail).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveDetail = resolve;
        }),
    );
    const note = entity();
    const { rerender } = render(<DecisionDocumentInspector entity={note} />);
    const signal = vi.mocked(api.getDocumentDetail).mock.calls[0][1]?.signal;
    rerender(<DecisionDocumentInspector entity={structuredClone(note)} />);
    expect(api.getDocumentDetail).toHaveBeenCalledTimes(1);
    expect(signal?.aborted).toBe(false);
    resolveDetail?.({ scope: { revision: 'retained-result' } });
    await screen.findAllByText(/retained-result/);
    rerender(<DecisionDocumentInspector entity={structuredClone(note)} />);
    for (const method of [
      api.getDocumentDetail,
      api.listIncomingDocumentDeclarations,
      api.listSubjectDecisions,
      api.listSubjectDocuments,
    ])
      expect(method).toHaveBeenCalledTimes(1);
    expect(signal?.aborted).toBe(false);
    expect(screen.getAllByText(/retained-result/).length).toBeGreaterThan(0);
  });
  it('aborts old queries and reloads on changed context, and suspends hydration for history', async () => {
    const note = entity();
    const { rerender } = render(<DecisionDocumentInspector entity={note} />);
    let count = 1;
    for (const changed of [
      { ...note, uid: 'new-incarnation' },
      { ...note, raw: { ...note.raw, title: { value: 'Edited' } } },
      { ...note, id: { ...id, version: '8' } },
    ]) {
      const signal = vi.mocked(api.getDocumentDetail).mock.calls.at(-1)?.[1]?.signal;
      rerender(<DecisionDocumentInspector entity={changed} />);
      expect(signal?.aborted).toBe(true);
      expect(api.getDocumentDetail).toHaveBeenCalledTimes(++count);
    }
    const signal = vi.mocked(api.getDocumentDetail).mock.calls.at(-1)?.[1]?.signal;
    window.history.replaceState({}, '', '/?branch=other');
    rerender(<DecisionDocumentInspector entity={note} />);
    expect(signal?.aborted).toBe(true);
    expect(api.getDocumentDetail).toHaveBeenLastCalledWith(
      { document: { id } },
      expect.objectContaining({ branch: 'other' }),
    );
    expect(api.getDocumentDetail).toHaveBeenCalledTimes(++count);
    const currentSignal = vi.mocked(api.getDocumentDetail).mock.calls.at(-1)?.[1]?.signal;
    rerender(<DecisionDocumentInspector entity={note} asOf="retained-revision" />);
    expect(currentSignal?.aborted).toBe(true);
    expect(api.getDocumentDetail).toHaveBeenCalledTimes(count);
    rerender(<DecisionDocumentInspector entity={note} />);
    await waitFor(() => expect(api.getDocumentDetail).toHaveBeenCalledTimes(count + 1));
  });
  it('recognizes kind and title, escapes retained Markdown and preserves exact branch/subject selectors', async () => {
    const note = entity();
    expect(note.kind).toBe('document');
    expect(note.title).toBe('Retained title');
    const { container } = render(<DecisionDocumentInspector entity={note} />);
    await waitFor(() => expect(api.getDocumentDetail).toHaveBeenCalled());
    expect(api.getDocumentDetail).toHaveBeenCalledWith(
      { document: { id } },
      expect.objectContaining({ branch: 'review', signal: expect.any(AbortSignal) }),
    );
    expect(api.listSubjectDocuments).toHaveBeenCalledWith(subject, expect.objectContaining({ branch: 'review' }));
    expect(container.querySelector('img,script,iframe,a')).toBeNull();
    expect([...container.querySelectorAll('pre')].some((node) => node.textContent === text)).toBe(true);
    expect(screen.getAllByText(/Incomplete inspection/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/Backend resolution failures/).length).toBeGreaterThan(0);
  });
  it('distinguishes whole, unpinned anchor and pinned anchor requests', async () => {
    const { unmount } = render(<DecisionDocumentInspector entity={entity()} />);
    await waitFor(() => expect(api.getDocumentDetail).toHaveBeenCalledTimes(1));
    const select = screen.getByLabelText('Exact document selection') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: select.options[1].value } });
    const anchor = { document: { id }, ownerUid: 'document-incarnation', anchorId: { value: 'anchor-b' } };
    await waitFor(() => expect(api.getDocumentDetail).toHaveBeenLastCalledWith({ anchor }, expect.anything()));
    fireEvent.change(select, { target: { value: select.options[2].value } });
    await waitFor(() =>
      expect(api.listIncomingDocumentDeclarations).toHaveBeenLastCalledWith(
        { anchor: { ...anchor, expectedDigest: digest } },
        expect.anything(),
      ),
    );
    const signal = vi.mocked(api.getDocumentDetail).mock.calls.at(-1)?.[1]?.signal;
    unmount();
    expect(signal?.aborted).toBe(true);
  });
  it('shows historical declarations without requesting current hydration', () => {
    render(<DecisionDocumentInspector entity={entity()} asOf="retained-revision" />);
    expect(screen.getByText(/Historical catalog hydration is unavailable/)).toBeTruthy();
    expect(api.getDocumentDetail).not.toHaveBeenCalled();
    expect(api.listSubjectDocuments).not.toHaveBeenCalled();
  });
  it('reports errors explicitly and keeps link-only content inert', async () => {
    vi.mocked(api.getDocumentDetail).mockRejectedValue(new Error('Not found'));
    const note = entity();
    delete note.raw.retained;
    note.raw.link = { value: 'https://example.test/note' };
    const { container } = render(<DecisionDocumentInspector entity={note} />);
    await waitFor(() => expect(screen.getByText(/Catalog inspection unavailable: Not found/)).toBeTruthy());
    expect(screen.getByText(/External link: unverified/)).toBeTruthy();
    expect(container.querySelector('a')).toBeNull();
  });
  it('does not recursively follow replacement chains by default', async () => {
    render(<DecisionDocumentInspector entity={entity('decision')} />);
    await waitFor(() =>
      expect(api.getDecisionDetail).toHaveBeenCalledWith({ id }, false, expect.objectContaining({ branch: 'review' })),
    );
  });
});

it('shows exact dated makers and paged amendments as inert backend evidence', async () => {
  const decision = entity('decision');
  decision.raw.authoredOn = { value: '2024-02-29' };
  decision.raw.decidedOn = { value: '2024-03-01' };
  decision.raw.decisionMakers = [
    { person: { id }, connectionId: { value: 'maker-occurrence' }, doc: { value: '<script>never()</script>' } },
  ];
  decision.raw.amends = [{ decision: { id }, connectionId: { value: 'amend-occurrence' } }];
  vi.mocked(api.listDecisionAmendments).mockResolvedValue({
    scope: { revision: 'amend-snapshot' },
    nextPageToken: 'opaque-page',
    declarations: [{ amendment: decision.raw.amends }],
  });
  render(<DecisionDocumentInspector entity={decision} />);
  await screen.findAllByText(/amend-snapshot/);
  expect(screen.getAllByText(/2024-02-29/).length).toBeGreaterThan(0);
  expect(screen.getAllByText(/maker-occurrence/).length).toBeGreaterThan(0);
  expect(document.querySelector('script')).toBeNull();
  expect(screen.getAllByRole('button', { name: 'Next amendments page' })).toHaveLength(2);
  fireEvent.click(screen.getAllByRole('button', { name: 'Next amendments page' })[0]);
  await waitFor(() =>
    expect(api.listDecisionAmendments).toHaveBeenCalledWith(
      { id },
      'DECISION_AMENDMENT_DIRECTION_OUTGOING',
      'opaque-page',
      expect.objectContaining({ branch: 'review' }),
    ),
  );
});

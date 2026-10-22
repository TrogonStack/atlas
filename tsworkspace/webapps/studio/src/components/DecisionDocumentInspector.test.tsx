import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from '@/lib/api';
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

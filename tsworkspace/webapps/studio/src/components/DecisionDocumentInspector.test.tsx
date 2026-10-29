import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import protobuf from 'protobufjs';
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
  it('labels page receipts without presenting their owners as the full scan', async () => {
    vi.mocked(api.listIncomingDocumentDeclarations).mockResolvedValue({
      scope: {
        revision: 'page-revision',
        pageReceipt: {
          checkedOwnerCount: '5000',
          totalDeclarations: '4097',
          startOffset: '0',
          endOffset: '100',
        },
      },
      nextPageToken: 'continuation',
      incomplete: false,
    });
    render(<DecisionDocumentInspector entity={entity()} />);
    expect(
      await screen.findByText(
        'Checked owners cover this page and the selected Document. The revision identifies the full checked scan.',
      ),
    ).toBeTruthy();
  });
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
  it('keeps retained binary and textual media inert without activating markup', async () => {
    const note = entity();
    delete note.raw.retained;
    note.raw.retainedContent = {
      mediaType: { value: 'text/html' },
      content: btoa('<svg onload="alert(1)"></svg>'),
      digest: { sha256: 'b'.repeat(64) },
    };
    const { container } = render(<DecisionDocumentInspector entity={note} />);
    await waitFor(() => expect(api.getDocumentDetail).toHaveBeenCalled());
    expect(container.querySelector('svg,script,iframe')).toBeNull();
    expect([...container.querySelectorAll('pre')].some((node) => node.textContent?.includes('<svg'))).toBe(true);
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

const protoSources = import.meta.glob('../../../../../proto/trogonatlas/{eventmodel,type}/v1alpha1/*.proto', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;
const catalogRoot = new protobuf.Root();
for (const wellKnown of ['any', 'duration', 'struct'])
  catalogRoot.addJSON(
    (protobuf.common.get(`google/protobuf/${wellKnown}.proto`) as { nested: Record<string, protobuf.AnyNestedObject> })
      .nested,
  );
for (const source of Object.values(protoSources)) protobuf.parse(source, catalogRoot);
catalogRoot.resolveAll();
const DocumentType = catalogRoot.lookupType('trogonatlas.eventmodel.v1alpha1.Document');

const digestOf = (bytes: Uint8Array) => (bytes.length ? bytes.length.toString(16).padStart(2, '0') : '00').repeat(32);
function contentEntity(
  content: unknown,
  mediaType = 'application/octet-stream',
  options: { uid?: string; filename?: unknown; digest?: unknown } = {},
) {
  const note = entity();
  delete note.raw.retained;
  note.uid = options.uid ?? 'document-incarnation';
  note.raw.retainedContent = {
    mediaType: { value: mediaType },
    content,
    digest: options.digest ?? { sha256: content instanceof Uint8Array ? digestOf(content) : 'c'.repeat(64) },
  };
  if (options.filename !== undefined) note.raw.filename = { value: options.filename };
  return note;
}
const b64 = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes));
const readBlob = (blob: Blob) =>
  new Promise<Uint8Array>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(new Uint8Array(reader.result as ArrayBuffer));
    reader.onerror = () => reject(reader.error);
    reader.readAsArrayBuffer(blob);
  });

describe('retained native content presentation', () => {
  const blobs = new Map<string, Blob>();
  const downloads: { name: string; href: string }[] = [];
  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;
  beforeEach(() => {
    blobs.clear();
    downloads.length = 0;
    let next = 0;
    URL.createObjectURL = vi.fn((blob: Blob) => {
      const url = `blob:atlas-test/${next++}`;
      blobs.set(url, blob);
      return url;
    });
    URL.revokeObjectURL = vi.fn();
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      downloads.push({ name: this.download, href: this.href });
    });
  });
  afterEach(() => {
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
  });
  const downloadButton = () => screen.queryByRole('button', { name: /Download exact bytes/ });
  async function downloadedBytes() {
    fireEvent.click(downloadButton() as HTMLElement);
    const last = downloads.at(-1);
    return { ...last, bytes: await readBlob(blobs.get(last?.href ?? '') as Blob), blob: blobs.get(last?.href ?? '') };
  }

  it('presents recognized text from base64 JSON as an inert pre and downloads the exact bytes', async () => {
    const literal = 'caf\u00e9 \u20ac\r\nline\0nul\n<script>alert(1)</script>\n';
    const bytes = new TextEncoder().encode(literal);
    const { container } = render(
      <DecisionDocumentInspector
        entity={contentEntity(b64(bytes), 'text/plain', { filename: 'fixture.txt' })}
        asOf="retained-revision"
      />,
    );
    const pres = [...container.querySelectorAll('pre')].filter((node) => node.textContent === literal);
    expect(pres).toHaveLength(1);
    expect(container.querySelector('script,svg,iframe,img,a')).toBeNull();
    const downloaded = await downloadedBytes();
    expect(downloaded.name).toBe('fixture.txt');
    expect([...downloaded.bytes]).toEqual([...bytes]);
    expect(downloaded.blob?.type).toBe('application/octet-stream');
  });

  it('presents and downloads a real generated-codec Document in both decoded shapes', async () => {
    const bytes = Uint8Array.from([0x68, 0x00, 0x69, 0x0a, 0xe2, 0x82, 0xac]);
    const message = DocumentType.fromObject({
      id,
      filename: { value: 'codec.json' },
      retainedContent: {
        mediaType: { value: 'application/json' },
        content: bytes,
        digest: { sha256: digestOf(bytes) },
      },
    });
    const wire = DocumentType.decode(DocumentType.encode(message).finish());
    for (const raw of [DocumentType.toObject(wire), DocumentType.toObject(wire, { bytes: String })]) {
      const note = entity();
      delete note.raw.retained;
      Object.assign(note.raw, raw);
      const { container, unmount } = render(<DecisionDocumentInspector entity={note} asOf="retained-revision" />);
      expect([...container.querySelectorAll('pre')].some((node) => node.textContent === 'h\0i\n\u20ac')).toBe(true);
      expect([...(await downloadedBytes()).bytes]).toEqual([...bytes]);
      expect(downloads.at(-1)?.name).toBe('codec.json');
      unmount();
    }
    expect(ArrayBuffer.isView(DocumentType.toObject(wire).retainedContent.content)).toBe(true);
  });

  it('treats binary and non-UTF-8 text media as download-only and keeps every byte', async () => {
    const bytes = Uint8Array.from([0xff, 0x00, 0xfe, 0x0d, 0x0a, 0x0a, 0x80]);
    for (const mediaType of ['application/octet-stream', 'text/plain']) {
      const { container, unmount } = render(
        <DecisionDocumentInspector entity={contentEntity(bytes, mediaType)} asOf="retained-revision" />,
      );
      expect(container.textContent).toContain(`${bytes.length} retained bytes`);
      expect(container.textContent).not.toContain('\ufffd');
      expect(screen.getByText(/Download only/)).toBeTruthy();
      const downloaded = await downloadedBytes();
      expect([...downloaded.bytes]).toEqual([...bytes]);
      expect(downloaded.name).toMatch(/^[A-Za-z0-9._-]+$/);
      unmount();
    }
  });

  it('never renders HTML or SVG actively', async () => {
    for (const mediaType of ['text/html', 'image/svg+xml', 'application/xhtml+xml']) {
      const markup = '<svg onload="alert(1)"><script>x()</script></svg><a href="javascript:x()">x</a><img src=x>';
      const { container, unmount } = render(
        <DecisionDocumentInspector
          entity={contentEntity(new TextEncoder().encode(markup), mediaType)}
          asOf="retained-revision"
        />,
      );
      expect(container.querySelector('svg,script,iframe,img,a,object,embed')).toBeNull();
      expect([...container.querySelectorAll('pre')].some((node) => node.textContent === markup)).toBe(true);
      expect((await downloadedBytes()).blob?.type).toBe('application/octet-stream');
      unmount();
    }
  });

  it.each([
    ['malformed base64', '***not base64***'],
    ['non-canonical base64', 'QR=='],
    ['wrong runtime type', 12],
    ['numeric array', [1, 2, 3]],
    ['empty bytes', new Uint8Array()],
    ['empty base64', ''],
    ['oversized bytes', new Uint8Array(256 * 1024 + 1)],
    ['missing content', undefined],
  ])('reports an explicit projection error for %s', (_name, content) => {
    const { container } = render(
      <DecisionDocumentInspector entity={contentEntity(content)} asOf="retained-revision" />,
    );
    expect(screen.getByRole('alert').textContent).toMatch(/Retained content cannot be presented/);
    expect(downloadButton()).toBeNull();
    expect(container.querySelector('pre:not(:first-of-type)')).toBeNull();
  });

  it.each([
    ['parameters', 'text/plain; charset=utf-8'],
    ['uppercase', 'Text/Plain'],
    ['no subtype', 'text'],
    ['blank', ''],
    ['too long', `application/${'a'.repeat(120)}`],
    ['non ASCII', 'text/pl\u00e4in'],
  ])('reports invalid media type metadata (%s)', (_name, mediaType) => {
    render(<DecisionDocumentInspector entity={contentEntity(Uint8Array.of(1), mediaType)} asOf="retained-revision" />);
    expect(screen.getByRole('alert').textContent).toMatch(/media type/);
    expect(downloadButton()).toBeNull();
  });

  it.each([
    ['short', { sha256: 'abc' }],
    ['uppercase', { sha256: 'A'.repeat(64) }],
    ['missing', undefined],
    ['wrong type', 'a'.repeat(64)],
  ])('reports invalid digest metadata (%s)', (_name, digest) => {
    const note = contentEntity(Uint8Array.of(1), 'application/octet-stream');
    (note.raw.retainedContent as Record<string, unknown>).digest = digest;
    render(<DecisionDocumentInspector entity={note} asOf="retained-revision" />);
    expect(screen.getByRole('alert').textContent).toMatch(/digest/);
    expect(downloadButton()).toBeNull();
  });

  it('keeps the declared digest, media type and filename visible exactly', () => {
    const bytes = Uint8Array.of(1, 2, 3);
    const { container } = render(
      <DecisionDocumentInspector
        entity={contentEntity(bytes, 'application/x-example', { filename: 'ex ample.bin' })}
        asOf="retained-revision"
      />,
    );
    for (const literal of [digestOf(bytes), 'application/x-example', 'ex ample.bin'])
      expect(container.textContent).toContain(literal);
  });

  it.each([
    ['slash', 'a/b.bin'],
    ['backslash', 'a\\b.bin'],
    ['dot', '.'],
    ['dot-dot', '..'],
    ['blank', '   '],
    ['control', 'a\nb.bin'],
    ['delete', 'a\u007fb.bin'],
    ['too long', 'a'.repeat(257)],
    ['multibyte too long', '\u20ac'.repeat(86)],
    ['wrong type', 7],
  ])('falls back to a safe download name for an invalid logical filename (%s)', async (_name, filename) => {
    render(
      <DecisionDocumentInspector
        entity={contentEntity(Uint8Array.of(9), undefined, { filename })}
        asOf="retained-revision"
      />,
    );
    const downloaded = await downloadedBytes();
    expect(downloaded.name).toBe('retained-content.bin');
    expect(screen.getByText(/Declared filename is not a valid logical filename/)).toBeTruthy();
  });

  it.each([{}, [], 'unexpected-unwrapped-name.bin'])('reports malformed filename wrappers (%j)', async (filename) => {
    const note = contentEntity(Uint8Array.of(9));
    note.raw.filename = filename;
    render(<DecisionDocumentInspector entity={note} asOf="retained-revision" />);
    expect(screen.getByText(/Declared filename is not a valid logical filename/)).toBeTruthy();
    expect((await downloadedBytes()).name).toBe('retained-content.bin');
  });

  it('accepts a 256 byte logical filename and uses the fallback when none is declared', async () => {
    const name = 'n'.repeat(256);
    const { unmount } = render(
      <DecisionDocumentInspector
        entity={contentEntity(Uint8Array.of(9), undefined, { filename: name })}
        asOf="retained-revision"
      />,
    );
    expect((await downloadedBytes()).name).toBe(name);
    unmount();
    render(<DecisionDocumentInspector entity={contentEntity(Uint8Array.of(9))} asOf="retained-revision" />);
    expect((await downloadedBytes()).name).toBe('retained-content.bin');
    expect(screen.queryByText(/not a valid logical filename/)).toBeNull();
  });

  it('never keeps content from a previous owner incarnation, selection or content', async () => {
    const first = new TextEncoder().encode('first owner');
    const second = new TextEncoder().encode('second owner');
    const { container, rerender } = render(
      <DecisionDocumentInspector
        entity={contentEntity(first, 'text/plain', { uid: 'owner-a' })}
        asOf="retained-revision"
      />,
    );
    expect(container.textContent).toContain('first owner');
    rerender(
      <DecisionDocumentInspector
        entity={contentEntity(second, 'text/plain', { uid: 'owner-b' })}
        asOf="retained-revision"
      />,
    );
    expect(container.textContent).not.toContain('first owner');
    expect([...(await downloadedBytes()).bytes]).toEqual([...second]);
    const note = contentEntity(second, 'text/plain', { uid: 'owner-b' });
    delete note.raw.retainedContent;
    note.raw.retained = { mediaType: { value: 'text/markdown' }, content: 'plain retained text', digest };
    rerender(<DecisionDocumentInspector entity={note} asOf="retained-revision" />);
    expect(container.textContent).not.toContain('second owner');
    expect(downloadButton()).toBeNull();
    const linkOnly = entity();
    delete linkOnly.raw.retained;
    rerender(<DecisionDocumentInspector entity={linkOnly} asOf="retained-revision" />);
    expect(downloadButton()).toBeNull();
    expect(container.textContent).not.toContain('retained bytes');
  });

  it('revokes pending object URLs immediately when the owner incarnation changes', () => {
    vi.useFakeTimers();
    try {
      const note = contentEntity(Uint8Array.of(1, 2));
      const view = render(<DecisionDocumentInspector entity={note} asOf="retained-revision" />);
      fireEvent.click(downloadButton() as HTMLElement);
      const url = vi.mocked(URL.createObjectURL).mock.results[0].value;
      expect(URL.revokeObjectURL).not.toHaveBeenCalled();
      view.rerender(
        <DecisionDocumentInspector entity={{ ...note, uid: 'replacement-incarnation' }} asOf="retained-revision" />,
      );
      expect(URL.revokeObjectURL).toHaveBeenCalledExactlyOnceWith(url);
      vi.runAllTimers();
      expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('revokes pending object URLs after use and when unmounted', async () => {
    vi.useFakeTimers();
    try {
      const view = render(
        <DecisionDocumentInspector entity={contentEntity(Uint8Array.of(1, 2))} asOf="retained-revision" />,
      );
      fireEvent.click(downloadButton() as HTMLElement);
      expect(URL.revokeObjectURL).not.toHaveBeenCalled();
      vi.runAllTimers();
      expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
      fireEvent.click(downloadButton() as HTMLElement);
      const second = vi.mocked(URL.createObjectURL).mock.results[1].value;
      view.unmount();
      expect(URL.revokeObjectURL).toHaveBeenLastCalledWith(second);
      vi.runAllTimers();
      expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not mutate or copy the authored raw content', () => {
    const bytes = Uint8Array.of(1, 2, 3);
    const note = contentEntity(bytes);
    note.raw.purpose = 'DOCUMENT_PURPOSE_GENERAL';
    const before = structuredClone(note.raw);
    const { container } = render(<DecisionDocumentInspector entity={note} asOf="retained-revision" />);
    expect(note.raw.retainedContent).toMatchObject({ content: bytes });
    expect(structuredClone(note.raw)).toEqual(before);
    expect(container.textContent).toContain('DOCUMENT_PURPOSE_GENERAL');
    expect(container.textContent).not.toContain('"0": 1');
  });
});

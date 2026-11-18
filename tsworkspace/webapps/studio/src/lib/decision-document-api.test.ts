import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getDecisionDetail,
  listDecisionAmendments,
  getDocumentDetail,
  listIncomingDecisionDeclarations,
  listIncomingDocumentDeclarations,
  listSubjectDecisions,
  listSubjectDocuments,
} from './api';

const id = { namespace: 'catalog', slug: 'note', version: '9007199254740993' };
const subject = {
  member: {
    specification: { id },
    ownerUid: 'spec-uid',
    kind: 'CONTRACT_MEMBER_KIND_ARTIFACT',
    memberId: { value: 'artifact-b' },
    expectedDigest: { sha256: 'a'.repeat(64) },
  },
};
const document = {
  anchor: {
    document: { id },
    ownerUid: 'doc-uid',
    anchorId: { value: 'anchor-b' },
    expectedDigest: { sha256: 'b'.repeat(64) },
  },
};
afterEach(() => vi.unstubAllGlobals());
describe('typed native catalog HTTP requests', () => {
  it('preserves selector/pin presence and checked results for every read-only query', async () => {
    const result = {
      incomplete: true,
      scope: { revision: 'checked', visitedDocuments: [document] },
      failures: [{ code: 14, message: 'Unavailable' }],
    };
    const fetch = vi
      .fn()
      .mockImplementation(() => Promise.resolve(new Response(JSON.stringify(result), { status: 200 })));
    vi.stubGlobal('fetch', fetch);
    const opts = { branch: 'review' };
    for (const call of [
      () => listSubjectDecisions(subject, undefined, opts),
      () => listSubjectDocuments(subject, opts),
      () => getDecisionDetail({ id }, false, opts),
      () => getDocumentDetail(document, opts),
      () => listIncomingDecisionDeclarations({ id }, opts),
      () => listIncomingDocumentDeclarations(document, opts),
    ])
      await expect(call()).resolves.toEqual(result);
    const inputs = fetch.mock.calls.map(([url, options]) => {
      const parsed = new URL(url, 'http://localhost');
      expect(parsed.searchParams.get('branch')).toBe('review');
      expect(options.headers['x-trogon-atlas-branch']).toBe('review');
      expect(options.method).toBeUndefined();
      return JSON.parse(parsed.searchParams.get('request') ?? '');
    });
    expect(inputs).toEqual([
      { subject },
      { subject },
      { decision: { id }, followReplacement: false },
      { document },
      { decision: { id } },
      { document },
    ]);
  });
  it('retains backend failure rather than manufacturing complete empty declarations', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: 'budget exhausted' }), { status: 429 })),
    );
    await expect(getDocumentDetail(document)).rejects.toThrow('budget exhausted');
  });
});

it('preserves amendment direction, opaque page context and branch', async () => {
  const fetch = vi
    .fn()
    .mockResolvedValue(new Response(JSON.stringify({ nextPageToken: 'next', incomplete: false }), { status: 200 }));
  vi.stubGlobal('fetch', fetch);
  await listDecisionAmendments({ id }, 'DECISION_AMENDMENT_DIRECTION_INCOMING', 'opaque-page', { branch: 'review' });
  const url = new URL(fetch.mock.calls[0][0], 'http://localhost');
  expect(url.pathname).toBe('/api/catalog/decision-amendments');
  expect(JSON.parse(url.searchParams.get('request') ?? '')).toEqual({
    decision: { id },
    direction: 'DECISION_AMENDMENT_DIRECTION_INCOMING',
    pageSize: 100,
    pageToken: 'opaque-page',
  });
  expect(url.searchParams.get('branch')).toBe('review');
});

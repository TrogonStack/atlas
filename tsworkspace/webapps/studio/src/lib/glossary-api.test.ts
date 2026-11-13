import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  compareGlossaries,
  getGlossaryTerm,
  getGlossaryUsage,
  inspectGlossaryRefresh,
  inspectGlossarySources,
  listGlossaryEntries,
} from './api';

const id = { namespace: 'sales', slug: 'order', version: '9007199254740993' };
const selection = { namespaces: ['sales'], audience: 'customer', includeUnreviewed: true };

afterEach(() => vi.unstubAllGlobals());

describe('glossary read-only HTTP requests', () => {
  it('preserves exact selection, version, paging and backend incomplete state', async () => {
    const result = { incomplete: true, snapshotRevision: 'checked', nextPageToken: 'opaque' };
    const fetch = vi.fn().mockImplementation(() => Promise.resolve(new Response(JSON.stringify(result))));
    vi.stubGlobal('fetch', fetch);
    const options = { branch: 'review' };
    const requests = [
      ['entries', { selection, pageSize: 100, pageToken: 'page' }],
      ['term', { term: { id } }],
      ['sources', { documents: [{ id }] }],
      ['compare', { left: selection, right: { namespaces: ['shipping'] } }],
      ['refresh', { selection, documents: [{ id }], generationRevision: 'extract-v1' }],
      ['usage', { kind: 'ENTITY_KIND_COMMAND', id }],
    ] as const;
    const calls = [
      () => listGlossaryEntries(requests[0][1], options),
      () => getGlossaryTerm(requests[1][1], options),
      () => inspectGlossarySources(requests[2][1], options),
      () => compareGlossaries(requests[3][1], options),
      () => inspectGlossaryRefresh(requests[4][1], options),
      () => getGlossaryUsage(requests[5][1], options),
    ];
    for (const call of calls) await expect(call()).resolves.toEqual(result);
    fetch.mock.calls.forEach(([input, init], index) => {
      const url = new URL(input, 'http://localhost');
      expect(url.pathname).toBe(`/api/glossary/${requests[index][0]}`);
      expect(url.searchParams.get('branch')).toBe('review');
      expect(JSON.parse(url.searchParams.get('request') ?? '')).toEqual(requests[index][1]);
      expect(init.method).toBeUndefined();
      expect(init.headers['x-trogon-atlas-branch']).toBe('review');
    });
  });

  it('retains failed inspection rather than manufacturing empty results', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: 'source unavailable' }), { status: 503 })),
    );
    await expect(getGlossaryTerm({ term: { id } })).rejects.toThrow('source unavailable');
  });
});

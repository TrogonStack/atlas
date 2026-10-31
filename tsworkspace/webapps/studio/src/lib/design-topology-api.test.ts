import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getDesignGraphDetail,
  listIncomingDesignGraphSubflows,
  listSubjectDesignGraphs,
  traverseDesignGraphTopology,
} from './api';

const id = { namespace: 'catalog', slug: 'checkout-flow', version: '9007199254740993' };
const subject = { entity: { kind: 'ENTITY_KIND_COMPONENT', id } };
afterEach(() => vi.unstubAllGlobals());
describe('typed design topology HTTP requests', () => {
  it('sends exact read-only requests on the matching catalog routes', async () => {
    const result = { incomplete: true, scope: { revision: 'checked' } };
    const fetch = vi
      .fn()
      .mockImplementation(() => Promise.resolve(new Response(JSON.stringify(result), { status: 200 })));
    vi.stubGlobal('fetch', fetch);
    const opts = { branch: 'review' };
    for (const call of [
      () => getDesignGraphDetail({ id }, opts),
      () => listSubjectDesignGraphs(subject, opts),
      () => traverseDesignGraphTopology({ id }, { followSubflows: true, maxDepth: 3 }, opts),
      () => traverseDesignGraphTopology({ id }, {}, opts),
      () => listIncomingDesignGraphSubflows({ id }, opts),
    ])
      await expect(call()).resolves.toEqual(result);
    const calls = fetch.mock.calls.map(([url, options]) => {
      const parsed = new URL(url, 'http://localhost');
      expect(parsed.searchParams.get('branch')).toBe('review');
      expect(options.method).toBeUndefined();
      return [parsed.pathname, JSON.parse(parsed.searchParams.get('request') ?? '')];
    });
    expect(calls).toEqual([
      ['/api/catalog/design-graph-detail', { graph: { id } }],
      ['/api/catalog/subject-design-graphs', { subject }],
      ['/api/catalog/design-topology', { root: { id }, followSubflows: true, maxDepth: 3 }],
      ['/api/catalog/design-topology', { root: { id } }],
      ['/api/catalog/incoming-subflows', { graph: { id } }],
    ]);
  });
});

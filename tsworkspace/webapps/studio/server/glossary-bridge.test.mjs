import request from 'supertest';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

const fixture = vi.hoisted(() => ({ calls: [], result: {}, failure: null }));
vi.mock('@grpc/grpc-js', async (importOriginal) => {
  const actual = await importOriginal();
  const client = { getServerInfo: (_req, _meta, _opts, cb) => cb(null, {}) };
  for (const method of ['listGlossaryEntries', 'getGlossaryTerm', 'inspectGlossarySources', 'compareGlossaries', 'inspectGlossaryRefresh', 'reviewGlossaryTerm', 'getEntity']) {
    client[method] = (input, metadata, _opts, callback) => {
      fixture.calls.push({ method, input, branch: metadata.get('x-trogon-atlas-branch')[0], authorization: metadata.get('authorization')[0] });
      callback(fixture.failure, fixture.result);
    };
  }
  const mocked = {
    ...actual,
    loadPackageDefinition: () => ({ trogonatlas: { api: { eventmodel: { v1alpha1: { EventModelService: function () { return client; } } } } } }),
  };
  return { ...mocked, default: mocked };
});

let app;
const oldPassthrough = process.env.TROGON_ATLAS_AUTH_PASSTHROUGH;
beforeAll(async () => {
  process.env.NODE_ENV = 'test';
  process.env.TROGON_ATLAS_AUTH_PASSTHROUGH = 'true';
  ({ app } = await import('./index.mjs'));
});
afterAll(() => {
  if (oldPassthrough === undefined) delete process.env.TROGON_ATLAS_AUTH_PASSTHROUGH;
  else process.env.TROGON_ATLAS_AUTH_PASSTHROUGH = oldPassthrough;
});
beforeEach(() => {
  fixture.calls.length = 0;
  fixture.result = { incomplete: true, snapshotRevision: 'checked-snapshot' };
  fixture.failure = null;
});

const id = { namespace: 'sales', slug: 'order', version: '9007199254740993' };
const selection = { namespaces: ['sales'], audience: 'customer', includeUnreviewed: true };
const reads = [
  ['entries', 'listGlossaryEntries', { selection, pageSize: 1, pageToken: 'opaque' }],
  ['term', 'getGlossaryTerm', { term: { id } }],
  ['sources', 'inspectGlossarySources', { documents: [{ id }] }],
  ['compare', 'compareGlossaries', { left: selection, right: { namespaces: ['shipping'] } }],
  ['refresh', 'inspectGlossaryRefresh', { selection, documents: [{ id }], generationRevision: 'extract-v1' }],
  ['usage', 'getEntity', { kind: 'ENTITY_KIND_COMMAND', id }],
];

describe('read-only glossary gateway', () => {
  it.each(reads)('forwards %s with exact request, caller and branch', async (query, method, input) => {
    const result = await request(app).get(`/api/glossary/${query}`).query({ request: JSON.stringify(input), branch: 'review' }).set('Authorization', 'Bearer fixture-reader');
    expect(result.status).toBe(200);
    expect(result.body).toEqual(fixture.result);
    expect(fixture.calls).toEqual([{ method, input, branch: 'review', authorization: 'Bearer fixture-reader' }]);
  });

  it.each([
    ['entries', { selection: { namespaces: ['sales'], includeUnreviewed: 'false' } }],
    ['entries', { selection: { namespaces: 'sales' } }],
    ['entries', { selection, page_size: 100 }],
    ['entries', { selection, pageSize: -1 }],
    ['term', { term: { id: { ...id, version: 9007199254740992 } } }],
    ['sources', { documents: [{ id: { ...id, version: '18446744073709551616' } }] }],
    ['compare', { left: selection, right: { namespaces: [false] } }],
    ['refresh', { selection, surprise: true }],
    ['usage', { kind: 'ENTITY_KIND_COMMAND', id: { ...id, version: '0' } }],
    ['usage', { kind: 'ENTITY_KIND_COMMAND' }],
    ['usage', { kind: 'UNKNOWN_KIND', id }],
  ])('rejects malformed %s before serializer coercion', async (query, input) => {
    const result = await request(app).get(`/api/glossary/${query}`).query({ request: JSON.stringify(input) }).set('Authorization', 'Bearer fixture-reader');
    expect(result.status).toBe(400);
    expect(fixture.calls).toEqual([]);
  });

  it('does not expose a review mutation or historical glossary reads', async () => {
    const review = await request(app).get('/api/glossary/review').query({ request: '{}' }).set('Authorization', 'Bearer fixture-reader');
    expect(review.status).toBe(400);
    const history = await request(app).get('/api/glossary/entries').query({ request: JSON.stringify({ selection }), asOf: 'past' }).set('Authorization', 'Bearer fixture-reader');
    expect(history.status).toBe(400);
    const mutation = await request(app).post('/api/glossary/term').send({ term: { id } });
    expect(mutation.status).toBe(403);
    expect(fixture.calls).toEqual([]);
  });

  it('preserves upstream failure rather than serving an empty catalog', async () => {
    fixture.failure = Object.assign(new Error('snapshot unavailable'), { code: 14 });
    const result = await request(app).get('/api/glossary/entries').query({ request: JSON.stringify({ selection }) }).set('Authorization', 'Bearer fixture-reader');
    expect(result.status).toBe(503);
    expect(result.body.error).toContain('snapshot unavailable');
  });
});

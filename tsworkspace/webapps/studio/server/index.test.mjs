import { beforeAll, describe, expect, it, vi } from 'vitest';
import request from 'supertest';

vi.mock('@grpc/proto-loader', () => {
  return {
    default: {
      loadSync: () => ({}),
    },
    loadSync: () => ({}),
  };
});

/** Last request each registry mutation forwarded upstream. */
const upstreamCalls = {
  registerNamespace: null,
  moveNamespace: null,
  deleteByQuery: null,
  putEntity: null,
  compileTypeLibrary: null,
  getTypeLibraryDescriptorSet: null,
  authorization: null,
};

const fakeClient = {
  getServerInfo: (_req, _meta, _opts, cb) => cb(null, { schemaVersion: '1', serverVersion: 'test' }),
  listEntityKinds: (_req, _meta, _opts, cb) => cb(null, { kinds: [] }),
  listValidationRules: (_req, _meta, _opts, cb) => cb(null, { rules: [] }),
  listEntities: (_req, _meta, _opts, cb) => cb(null, { entities: [], nextPageToken: '' }),
  searchEntities: (_req, _meta, _opts, cb) => cb(null, { results: [] }),
  listChanges: (_req, _meta, _opts, cb) => cb(null, { events: [], nextToken: '' }),
  validateEventModel: (_req, _meta, _opts, cb) => cb(null, { issues: [] }),
  validateProject: (_req, _meta, _opts, cb) => cb(null, { reports: [], totalErrors: 0, totalWarnings: 0, totalInfo: 0 }),
  getEntity: (_req, _meta, _opts, cb) => cb(null, { entity: null }),
  listEntitiesByDomain: (_req, _meta, _opts, cb) => cb(null, { entities: [] }),
  listNamespaces: (_req, meta, _opts, cb) => {
    upstreamCalls.authorization = meta?.get('authorization')?.[0] ?? null;
    cb(null, { namespaces: [] });
  },
  registerNamespace: (req, meta, _opts, cb) => {
    upstreamCalls.registerNamespace = req;
    upstreamCalls.authorization = meta?.get('authorization')?.[0] ?? null;
    cb(null, {
      namespace: { id: 'ns_0192abcd', name: req.name, parent: req.parent || 'default' },
      created: true,
    });
  },
  moveNamespace: (req, _meta, _opts, cb) => {
    upstreamCalls.moveNamespace = req;
    cb(null, { namespace: { id: req.id, name: 'billing', parent: req.parent } });
  },
  deleteByQuery: (req, _meta, _opts, cb) => {
    upstreamCalls.deleteByQuery = req;
    cb(null, { deleted: [], deletedCount: 0 });
  },
  putEntity: (req, _meta, _opts, cb) => {
    upstreamCalls.putEntity = req;
    cb(null, {});
  },
  compileTypeLibrary: (req, meta, _opts, cb) => {
    upstreamCalls.compileTypeLibrary = { req, branch: meta?.get('x-trogon-atlas-branch')?.[0] ?? null };
    cb(null, { diagnostics: [{ path: 'acme/v1/a.proto', line: 3, column: 7, message: 'syntax error' }] });
  },
  getTypeLibraryDescriptorSet: (req, _meta, _opts, cb) => {
    upstreamCalls.getTypeLibraryDescriptorSet = req;
    cb(null, {
      fileDescriptorSet: Buffer.from([1, 2, 3]),
      messages: [{ fullName: 'acme.v1.Money', library: { namespace: 'acme', slug: 'acme.v1', version: '1' } }],
    });
  },
  listBranches: (_req, _meta, _opts, cb) =>
    cb(null, {
      branches: [
        { name: 'alex/retention-rework', doc: 'in progress', createdAt: '2026-01-01T00:00:00Z', deltaCount: 3 },
      ],
    }),
  diffBranch: (_req, _meta, _opts, cb) =>
    cb(null, {
      entries: [
        {
          ref: { kind: 'ENTITY_KIND_EVENT', id: { namespace: 'my-ns', slug: 'my-event', version: '1' } },
          status: 'STATUS_ADDED',
          base: null,
          baseEtag: '',
          ours: null,
          theirs: null,
          conflictFieldPaths: [],
        },
      ],
    }),
};

vi.mock('@grpc/grpc-js', async (importOriginal) => {
  const actual = await importOriginal();
  const mocked = {
    ...actual,
    loadPackageDefinition: () => ({
      trogonatlas: {
        api: {
          eventmodel: {
            v1alpha1: {
              EventModelService: function () {
                return fakeClient;
              },
            },
          },
        },
      },
    }),
    credentials: {
      createInsecure: () => ({}),
      createSsl: () => ({}),
    },
    Metadata: actual.Metadata,
  };
  return { ...mocked, default: mocked };
});

vi.mock('protobufjs', () => {
  const fakeEntityKindValues = {
    ENTITY_KIND_UNSPECIFIED: 0, ENTITY_KIND_EVENT: 1, ENTITY_KIND_COMMAND: 2,
    ENTITY_KIND_READ_MODEL: 3, ENTITY_KIND_PROCESSOR: 4, ENTITY_KIND_UI: 5,
    ENTITY_KIND_PERSONA: 6, ENTITY_KIND_SWIMLANE: 7, ENTITY_KIND_COMMAND_SLICE: 8,
    ENTITY_KIND_READ_MODEL_SLICE: 9, ENTITY_KIND_AUTOMATION_SLICE: 10,
    ENTITY_KIND_STORYBOARD: 11, ENTITY_KIND_EVENT_MODEL: 12,
    ENTITY_KIND_COMPONENT: 13, ENTITY_KIND_EXTERNAL_SYSTEM: 14,
    ENTITY_KIND_TRACKER: 15, ENTITY_KIND_BOUNDED_CONTEXT: 16,
    ENTITY_KIND_DOMAIN: 17, ENTITY_KIND_SUBDOMAIN: 18, ENTITY_KIND_SCHEMA: 19,
    ENTITY_KIND_PROJECT: 20, ENTITY_KIND_SCREEN: 21, ENTITY_KIND_TERM: 22,
    ENTITY_KIND_AMBIGUITY: 23,
  };
  const fakeRoot = {
    resolvePath: () => '',
    load: () => Promise.resolve(fakeRoot),
    lookupType: () => ({
      decode: () => ({}),
      toObject: () => ({}),
    }),
    lookupEnum: (name) => {
      if (name === 'trogonatlas.eventmodel.v1alpha1.EntityKind') {
        return { values: fakeEntityKindValues };
      }
      throw new Error(`enum not found: ${name}`);
    },
  };
  return {
    default: {
      Root: class {
        constructor() {
          Object.assign(this, fakeRoot);
        }
      },
    },
  };
});

let app;
let call;

beforeAll(async () => {
  process.env.NODE_ENV = 'test';
  const mod = await import('./index.mjs');
  app = mod.app;
  call = mod.call;
});

describe('GET /api/info', () => {
  it('returns 200 with server info', async () => {
    const res = await request(app).get('/api/info');
    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({ schemaVersion: '1', serverVersion: 'test' });
  });
});

describe('Security headers', () => {
  it('sets X-Content-Type-Options', async () => {
    const res = await request(app).get('/api/info');
    expect(res.headers['x-content-type-options']).toBe('nosniff');
  });

  it('sets Content-Security-Policy', async () => {
    const res = await request(app).get('/api/info');
    expect(res.headers['content-security-policy']).toContain("default-src 'self'");
  });

  it('sets X-Frame-Options', async () => {
    const res = await request(app).get('/api/info');
    expect(res.headers['x-frame-options']).toBe('DENY');
  });

  it('sets Referrer-Policy', async () => {
    const res = await request(app).get('/api/info');
    expect(res.headers['referrer-policy']).toBe('no-referrer');
  });
});

describe('Rate limiter', () => {
  it('is applied to /api routes', async () => {
    const res = await request(app).get('/api/info');
    expect([200, 429]).toContain(res.status);
  });
});

describe('GET /api/nats-auth', () => {
  it('returns null token when env var is unset', async () => {
    delete process.env.NATS_WS_AUTH_TOKEN;
    const res = await request(app).get('/api/nats-auth');
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty('token');
  });
});

describe('Unknown routes', () => {
  it('returns 404 for unknown API routes', async () => {
    const res = await request(app).get('/api/does-not-exist-xyz-abc');
    expect(res.status).toBe(404);
  });
});

describe('GET /api/nats-auth auth-required behavior', () => {
  let authApp;

  beforeAll(async () => {
    // Load a fresh app instance with AUTH_TOKEN set so the nats-auth guard
    // is active. The query-string bust forces a new module evaluation.
    process.env.TROGON_ATLAS_AUTH_TOKEN = 'test-nats-secret';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=natsauth');
    authApp = mod.app;
    delete process.env.TROGON_ATLAS_AUTH_TOKEN;
  });

  it('returns 401 when no bearer token is sent', async () => {
    const res = await request(authApp).get('/api/nats-auth');
    expect(res.status).toBe(401);
    expect(res.body).toHaveProperty('error');
  });

  it('returns 401 when a wrong bearer token is sent', async () => {
    const res = await request(authApp)
      .get('/api/nats-auth')
      .set('Authorization', 'Bearer wrong-value');
    expect(res.status).toBe(401);
  });

  it('returns 200 and a token field when the correct bearer token is sent', async () => {
    const res = await request(authApp)
      .get('/api/nats-auth')
      .set('Authorization', 'Bearer test-nats-secret');
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty('token');
  });
});

describe('Path-param length validation', () => {
  const longParam = 'a'.repeat(513);

  it('returns 400 when namespace exceeds 512 chars on /api/event-model', async () => {
    const res = await request(app).get(`/api/event-model/${longParam}/slug`);
    expect(res.status).toBe(400);
  });

  it('returns 400 when slug exceeds 512 chars on /api/event-model', async () => {
    const res = await request(app).get(`/api/event-model/ns/${longParam}`);
    expect(res.status).toBe(400);
  });

  it('returns 400 when namespace exceeds 512 chars on /api/by-domain', async () => {
    const res = await request(app).get(`/api/by-domain/${longParam}/slug`);
    expect(res.status).toBe(400);
  });
});

describe('gRPC error masking', () => {
  it('strips bearer tokens from error responses', async () => {
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = (_req, _meta, _opts, cb) =>
      cb(
        Object.assign(new Error('authorization: Bearer leaked-secret downstream error'), {
          code: 2,
        }),
      );
    const res = await request(app).get('/api/info');
    expect(res.status).toBe(500);
    expect(JSON.stringify(res.body)).not.toContain('leaked-secret');
    expect(res.body).toHaveProperty('error');
    fakeClient.getServerInfo = original;
  });

  it('maps gRPC NOT_FOUND (code 5) to HTTP 404', async () => {
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = (_req, _meta, _opts, cb) =>
      cb(Object.assign(new Error('entity not found'), { code: 5 }));
    const res = await request(app).get('/api/info');
    expect(res.status).toBe(404);
    fakeClient.getServerInfo = original;
  });
});

describe('Global API auth middleware', () => {
  let authApp;

  beforeAll(async () => {
    process.env.TROGON_ATLAS_AUTH_TOKEN = 'global-secret';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=globalauth');
    authApp = mod.app;
    delete process.env.TROGON_ATLAS_AUTH_TOKEN;
  });

  it('allows /api/info without a token (public health endpoint)', async () => {
    const res = await request(authApp).get('/api/info');
    expect(res.status).toBe(200);
  });

  it('returns 401 on /api/model when no token is sent', async () => {
    const res = await request(authApp).get('/api/model');
    expect(res.status).toBe(401);
    expect(res.body).toHaveProperty('error');
  });

  it('returns 401 on /api/model when wrong token is sent', async () => {
    const res = await request(authApp)
      .get('/api/model')
      .set('Authorization', 'Bearer wrong');
    expect(res.status).toBe(401);
  });

  it('returns 200 on /api/model when correct token is sent', async () => {
    const res = await request(authApp)
      .get('/api/model')
      .set('Authorization', 'Bearer global-secret');
    expect(res.status).toBe(200);
  });

  it('passes all routes when AUTH_TOKEN is not configured', async () => {
    const res = await request(app).get('/api/model');
    expect(res.status).toBe(200);
  });
});

describe('Namespace query validation', () => {
  it('returns 400 when ?namespace= contains an invalid token', async () => {
    const res = await request(app).get('/api/model?namespace=../etc/passwd');
    expect(res.status).toBe(400);
  });

  it('returns 400 when ?namespace= contains shell metacharacters', async () => {
    const res = await request(app).get('/api/model?namespace=foo;bar');
    expect(res.status).toBe(400);
  });

  it('returns 400 when ?namespace= exceeds 4096 chars', async () => {
    const long = 'a'.repeat(4097);
    const res = await request(app).get(`/api/model?namespace=${long}`);
    expect(res.status).toBe(400);
  });

  it('returns 200 when ?namespace= is a valid token', async () => {
    const res = await request(app).get('/api/model?namespace=my-project_1.0');
    expect(res.status).toBe(200);
  });

  it('returns 400 on /api/search when ?namespace= is invalid', async () => {
    const res = await request(app).get('/api/search?q=foo&namespace=../../bad');
    expect(res.status).toBe(400);
  });
});

describe('SSE /api/changes/stream', () => {
  it('returns 401 when AUTH_TOKEN is configured and no token is sent', async () => {
    let authStreamApp;
    process.env.TROGON_ATLAS_AUTH_TOKEN = 'sse-secret';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=sseauth');
    authStreamApp = mod.app;
    delete process.env.TROGON_ATLAS_AUTH_TOKEN;

    const res = await request(authStreamApp).get('/api/changes/stream');
    expect(res.status).toBe(401);
  });

  it('enforces per-IP concurrency cap and returns 429 when exceeded', async () => {
    const originalMax = process.env.TROGON_ATLAS_STUDIO_SSE_MAX_PER_IP;
    process.env.TROGON_ATLAS_STUDIO_SSE_MAX_PER_IP = '0';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=ssecap');
    const capApp = mod.app;
    if (originalMax !== undefined) {
      process.env.TROGON_ATLAS_STUDIO_SSE_MAX_PER_IP = originalMax;
    } else {
      delete process.env.TROGON_ATLAS_STUDIO_SSE_MAX_PER_IP;
    }

    const res = await request(capApp).get('/api/changes/stream');
    expect(res.status).toBe(429);
  });

  it('decrements connection count after client disconnects so the slot is reusable', async () => {
    // Use a cap-of-1 app instance so we can verify slot reclamation.
    process.env.TROGON_ATLAS_STUDIO_SSE_MAX_PER_IP = '1';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=ssecleanup');
    const singleSlotApp = mod.app;
    delete process.env.TROGON_ATLAS_STUDIO_SSE_MAX_PER_IP;

    // First connection: open and immediately let it time out (simulates disconnect).
    await request(singleSlotApp)
      .get('/api/changes/stream')
      .timeout({ deadline: 150 })
      .catch(() => {});

    // Give the close event a tick to propagate.
    await new Promise((r) => setTimeout(r, 50));

    // Second connection from the same IP should succeed (slot was reclaimed).
    const res2 = await request(singleSlotApp)
      .get('/api/changes/stream')
      .timeout({ deadline: 150 })
      .catch((err) => err.response ?? { status: err.status });

    // 200 (stream started) or a timeout error (not 429) confirms the slot was freed.
    expect(res2?.status ?? 200).not.toBe(429);
  });
});

describe('GET /api/overview', () => {
  it('returns 200 with entities array', async () => {
    const res = await request(app).get('/api/overview');
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty('entities');
    expect(Array.isArray(res.body.entities)).toBe(true);
  });
});

describe('GET /api/event-models', () => {
  it('returns 200 with entities array', async () => {
    const res = await request(app).get('/api/event-models');
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty('entities');
    expect(Array.isArray(res.body.entities)).toBe(true);
  });
});

describe('GET /api/validate', () => {
  it('returns 200 with issues array for valid namespace and slug', async () => {
    const res = await request(app).get('/api/validate?namespace=my-ns&slug=my-model');
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty('issues');
  });

  it('returns 400 when namespace is unsafe', async () => {
    const res = await request(app).get('/api/validate?namespace=../etc/passwd&slug=my-model');
    expect(res.status).toBe(400);
  });

  it('returns 400 when slug is unsafe', async () => {
    const res = await request(app).get('/api/validate?namespace=my-ns&slug=;evil');
    expect(res.status).toBe(400);
  });
});

describe('GET /api/validate-project', () => {
  it('returns 400 when neither project nor domain params are provided', async () => {
    const res = await request(app).get('/api/validate-project');
    expect(res.status).toBe(400);
  });

  it('returns 200 with reports when valid project params are provided', async () => {
    const res = await request(app).get('/api/validate-project?projectNamespace=my-ns&projectSlug=my-project');
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty('reports');
  });

  it('returns 400 when projectNamespace is unsafe', async () => {
    const res = await request(app).get('/api/validate-project?projectNamespace=../bad&projectSlug=my-project');
    expect(res.status).toBe(400);
  });
});

describe('GET /api/event-model/:namespace/:slug returning 404 for missing entity', () => {
  it('returns 404 when gRPC returns entity: null', async () => {
    const res = await request(app).get('/api/event-model/my-ns/nonexistent-model');
    expect(res.status).toBe(404);
  });
});

describe('GET /api/event-model/:namespace/:slug success path (members, halo, knowledge graph)', () => {
  it('fetches members, adds a cross-namespace halo event, and includes the knowledge graph', async () => {
    const originalGetEntity = fakeClient.getEntity;
    fakeClient.getEntity = (req, _meta, _opts, cb) => {
      const { kind, id } = req;
      if (kind === 12 && id.namespace === 'my-ns' && id.slug === 'my-model') {
        cb(null, {
          entity: {
            eventModel: {
              id: { namespace: 'my-ns', slug: 'my-model', version: '1' },
              title: 'My model',
              members: [
                { kind: 'ENTITY_KIND_EVENT', id: { namespace: 'my-ns', slug: 'placed', version: '1' } },
                { kind: 'ENTITY_KIND_READ_MODEL', id: { namespace: 'my-ns', slug: 'summary', version: '1' } },
              ],
            },
          },
        });
        return;
      }
      if (kind === 1 && id.namespace === 'my-ns' && id.slug === 'placed') {
        cb(null, { entity: { event: { id, title: 'Placed' } } });
        return;
      }
      if (kind === 3 && id.namespace === 'my-ns' && id.slug === 'summary') {
        cb(null, {
          entity: {
            readModel: {
              id,
              title: 'Summary',
              sourceEvents: [{ id: { namespace: 'upstream-ns', slug: 'thing.done', version: '1' } }],
            },
          },
        });
        return;
      }
      if (kind === 1 && id.namespace === 'upstream-ns' && id.slug === 'thing.done') {
        cb(null, { entity: { event: { id, title: 'Thing done' } } });
        return;
      }
      if (kind === 16) {
        cb(null, {
          entity: {
            boundedContext: {
              id: { namespace: 'my-ns', slug: 'my-ns', version: '1' },
              title: 'My context',
              realizes: [{ id: { namespace: 'my-ns', slug: 'my-subdomain', version: '1' } }],
            },
          },
        });
        return;
      }
      if (kind === 18) {
        cb(null, {
          entity: {
            subdomain: {
              id: { namespace: 'my-ns', slug: 'my-subdomain', version: '1' },
              title: 'My subdomain',
              domain: { id: { namespace: 'my-ns', slug: 'my-domain', version: '1' } },
            },
          },
        });
        return;
      }
      if (kind === 17) {
        cb(null, {
          entity: {
            domain: { id: { namespace: 'my-ns', slug: 'my-domain', version: '1' }, title: 'My domain' },
          },
        });
        return;
      }
      cb(null, { entity: null });
    };
    const originalListEntities = fakeClient.listEntities;
    fakeClient.listEntities = (_req, _meta, _opts, cb) => cb(null, { entities: [], nextPageToken: '' });

    const res = await request(app).get('/api/event-model/my-ns/my-model');

    fakeClient.getEntity = originalGetEntity;
    fakeClient.listEntities = originalListEntities;

    expect(res.status).toBe(200);
    const kinds = res.body.entities.map((e) => Object.keys(e)[0]);
    expect(kinds).toContain('eventModel');
    expect(kinds).toContain('event');
    expect(kinds).toContain('readModel');
    // The cross-namespace halo event and the knowledge graph (bounded
    // context, subdomain, domain) all get merged into the response.
    expect(res.body.entities.some((e) => e.event?.id?.namespace === 'upstream-ns')).toBe(true);
    expect(res.body.entities.some((e) => e.boundedContext)).toBe(true);
    expect(res.body.entities.some((e) => e.subdomain)).toBe(true);
    expect(res.body.entities.some((e) => e.domain)).toBe(true);
  });

  it('tolerates a member fetch failure by dropping that member rather than failing the request', async () => {
    const originalGetEntity = fakeClient.getEntity;
    fakeClient.getEntity = (req, _meta, _opts, cb) => {
      const { kind, id } = req;
      if (kind === 12 && id.slug === 'flaky-model') {
        cb(null, {
          entity: {
            eventModel: {
              id: { namespace: 'my-ns', slug: 'flaky-model', version: '1' },
              title: 'Flaky model',
              members: [{ kind: 'ENTITY_KIND_EVENT', id: { namespace: 'my-ns', slug: 'boom', version: '1' } }],
            },
          },
        });
        return;
      }
      if (kind === 1 && id.slug === 'boom') {
        cb(new Error('upstream unavailable'));
        return;
      }
      if (kind === 16) {
        cb(null, { entity: null });
        return;
      }
      cb(null, { entity: null });
    };

    const res = await request(app).get('/api/event-model/my-ns/flaky-model');

    fakeClient.getEntity = originalGetEntity;

    expect(res.status).toBe(200);
    const kinds = res.body.entities.map((e) => Object.keys(e)[0]);
    expect(kinds).toEqual(['eventModel']);
  });

  it('tolerates a knowledge-graph fetch failure by returning the entities without it', async () => {
    const originalGetEntity = fakeClient.getEntity;
    fakeClient.getEntity = (req, _meta, _opts, cb) => {
      const { kind, id } = req;
      if (kind === 12 && id.slug === 'no-context-model') {
        cb(null, {
          entity: {
            eventModel: {
              id: { namespace: 'my-ns', slug: 'no-context-model', version: '1' },
              title: 'No context model',
              members: [],
            },
          },
        });
        return;
      }
      if (kind === 16) {
        cb(new Error('bounded context lookup exploded'));
        return;
      }
      cb(null, { entity: null });
    };

    const res = await request(app).get('/api/event-model/my-ns/no-context-model');

    fakeClient.getEntity = originalGetEntity;

    expect(res.status).toBe(200);
    expect(res.body.entities).toHaveLength(1);
    expect(res.body.entities[0].eventModel.id.slug).toBe('no-context-model');
  });
});

describe('GET /api/by-domain unsafe path param', () => {
  it('returns 400 when namespace contains unsafe characters', async () => {
    const res = await request(app).get('/api/by-domain/..%2Fetc%2Fpasswd/slug');
    expect(res.status).toBe(400);
  });

  it('returns 400 when slug contains unsafe characters', async () => {
    const res = await request(app).get('/api/by-domain/my-ns/;evil');
    expect(res.status).toBe(400);
  });
});

describe('GET /api/by-domain kinds query parity', () => {
  it('accepts dash and collapsed kind aliases like rust parse_kind', async () => {
    /** @type {unknown} */
    let captured = null;
    const original = fakeClient.listEntitiesByDomain;
    fakeClient.listEntitiesByDomain = (req, _meta, _opts, cb) => {
      captured = req;
      cb(null, { entities: [] });
    };

    const res = await request(app).get(
      '/api/by-domain/my-ns/checkout?kinds=read-model,commandslice,ENTITY_KIND_EVENT',
    );

    fakeClient.listEntitiesByDomain = original;

    expect(res.status).toBe(200);
    expect(captured).toMatchObject({
      kinds: expect.arrayContaining([3, 8, 1]),
    });
    expect(captured.kinds).toHaveLength(3);
  });

  it('returns 400 for unknown kind tokens instead of silently listing all', async () => {
    /** @type {unknown} */
    let captured = null;
    const original = fakeClient.listEntitiesByDomain;
    fakeClient.listEntitiesByDomain = (req, _meta, _opts, cb) => {
      captured = req;
      cb(null, { entities: [] });
    };

    const res = await request(app).get('/api/by-domain/my-ns/checkout?kinds=not-a-kind');

    fakeClient.listEntitiesByDomain = original;

    expect(res.status).toBe(400);
    expect(captured).toBeNull();
  });
});

describe('GET /api/search branch context', () => {
  it('forwards the branch so a preview searches the branch index', async () => {
    // service.proto used to say SearchEntities ignored this header, and the
    // gateway believed it. The server has had a per-branch index since.
    let meta = null;
    const original = fakeClient.searchEntities;
    fakeClient.searchEntities = (_req, m, _opts, cb) => {
      meta = m;
      cb(null, { results: [] });
    };
    try {
      const res = await request(app).get('/api/search?q=placed&branch=alex%2Fretention-rework');
      expect(res.status).toBe(200);
      expect(meta.get('x-trogon-atlas-branch')).toEqual(['alex/retention-rework']);
    } finally {
      fakeClient.searchEntities = original;
    }
  });
});

describe('GET /api/namespaces', () => {
  /** @param {unknown[]} namespaces @param {(res: import('supertest').Response, captured: {req: unknown, meta: any}) => void} check */
  async function withNamespaces(namespaces, url, check) {
    const original = fakeClient.listNamespaces;
    const captured = { req: null, meta: null };
    fakeClient.listNamespaces = (req, meta, _opts, cb) => {
      captured.req = req;
      captured.meta = meta;
      cb(null, { namespaces });
    };
    try {
      check(await request(app).get(url), captured);
    } finally {
      fakeClient.listNamespaces = original;
    }
  }

  it('asks the server rather than deriving the list from stored models', async () => {
    // The derived version could not see a namespace that had been claimed
    // through RegisterNamespace but did not hold an event model yet.
    await withNamespaces(
      [{ id: 'ns_1', name: 'claimed-but-empty', parent: 'acme', entityCount: 0 }],
      '/api/namespaces',
      (res, captured) => {
        expect(res.status).toBe(200);
        expect(captured.req).toEqual({});
        expect(res.body.namespaces).toEqual(['ns_1']);
      },
    );
  });

  it('sends ids and labels them with the human name', async () => {
    // ListEntities filters on the id, so the id is what travels; the name
    // is the only part a person can read.
    await withNamespaces(
      [
        { id: 'ns_0199', name: 'orders', parent: 'acme', entityCount: 4 },
        { id: 'legacy-ns', name: 'legacy-ns', parent: '', entityCount: 1 },
      ],
      '/api/namespaces',
      (res) => {
        expect(res.body.namespaces).toEqual(['legacy-ns', 'ns_0199']);
        expect(res.body.labels).toEqual({ ns_0199: 'orders', 'legacy-ns': 'legacy-ns' });
      },
    );
  });

  it('qualifies a name two owners share instead of printing it twice', async () => {
    await withNamespaces(
      [
        { id: 'ns_a', name: 'orders', parent: 'acme', entityCount: 1 },
        { id: 'ns_b', name: 'orders', parent: 'globex', entityCount: 2 },
      ],
      '/api/namespaces',
      (res) => {
        expect(res.body.labels).toEqual({ ns_a: 'orders (acme)', ns_b: 'orders (globex)' });
      },
    );
  });

  it('forwards the branch so a namespace introduced on a branch is offered', async () => {
    await withNamespaces([], '/api/namespaces?branch=alex%2Fretention-rework', (res, captured) => {
      expect(res.status).toBe(200);
      expect(captured.meta.get('x-trogon-atlas-branch')).toEqual(['alex/retention-rework']);
    });
  });
});

describe('GET /api/nats-auth with a closed listener behind an open bridge', () => {
  it('routes the feed through the bridge rather than publishing the listener credential', async () => {
    // This bridge authenticates nobody, so anything it returns here is
    // returned to anyone. The listener credential exists to stop exactly that
    // caller, and the operator who set it should still get a working feed.
    process.env.NATS_WS_AUTH_TOKEN = 'nats-secret';
    delete process.env.TROGON_ATLAS_AUTH_TOKEN;
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=nats503');
    const natsApp = mod.app;
    delete process.env.NATS_WS_AUTH_TOKEN;

    const res = await request(natsApp).get('/api/nats-auth');
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ token: null, transport: 'sse' });
    expect(JSON.stringify(res.body)).not.toContain('nats-secret');
  });
});

describe('CSP connect-src does not contain bare ws:/wss: wildcards', () => {
  it('default app has no bare ws: or wss: in connect-src', async () => {
    const res = await request(app).get('/api/info');
    const csp = res.headers['content-security-policy'] ?? '';
    // Extract the connect-src directive value.
    const match = csp.match(/connect-src\s+([^;]+)/);
    const connectSrc = match ? match[1] : '';
    // Must not contain standalone ws: or wss: (the scheme-only wildcard forms).
    // A specific origin like ws://nats.example.com:8080 is acceptable but not
    // tested here because the default app has no NATS WS URL configured.
    expect(connectSrc).not.toMatch(/\bws:\s*($|;|\s)/);
    expect(connectSrc).not.toMatch(/\bwss:\s*($|;|\s)/);
  });

  it('includes the configured NATS WS origin in connect-src when env var is set', async () => {
    process.env.TROGON_ATLAS_STUDIO_NATS_WS_URL = 'ws://nats.example.com:8080';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=natswsurl');
    const natsUrlApp = mod.app;
    delete process.env.TROGON_ATLAS_STUDIO_NATS_WS_URL;

    const res = await request(natsUrlApp).get('/api/info');
    const csp = res.headers['content-security-policy'] ?? '';
    expect(csp).toContain('ws://nats.example.com:8080');
    // Still must not contain bare ws: wildcard.
    const match = csp.match(/connect-src\s+([^;]+)/);
    const connectSrc = match ? match[1] : '';
    expect(connectSrc).not.toMatch(/\bws:\s*($|;|\s)/);
  });
});

describe('HSTS header', () => {
  it('is absent in test (non-production) mode', async () => {
    const res = await request(app).get('/api/info');
    expect(res.headers['strict-transport-security']).toBeUndefined();
  });

  it('is present with correct value in production mode', async () => {
    process.env.NODE_ENV = 'production';
    const mod = await import('./index.mjs?t=production');
    const prodApp = mod.app;

    // Keep NODE_ENV=production during the request because the middleware
    // reads process.env.NODE_ENV at request time, not at startup.
    const res = await request(prodApp).get('/api/info');

    process.env.NODE_ENV = 'test';

    expect(res.headers['strict-transport-security']).toBe(
      'max-age=63072000; includeSubDomains',
    );
  });
});

describe('?branch= query param', () => {
  it('forwards the branch as x-trogon-atlas-branch metadata on a read endpoint', async () => {
    let receivedMeta;
    const original = fakeClient.listEntities;
    fakeClient.listEntities = (_req, meta, _opts, cb) => {
      receivedMeta = meta;
      cb(null, { entities: [], nextPageToken: '' });
    };
    const res = await request(app).get('/api/model?branch=alex/retention-rework');
    fakeClient.listEntities = original;
    expect(res.status).toBe(200);
    expect(receivedMeta?.get?.('x-trogon-atlas-branch')).toEqual(['alex/retention-rework']);
  });

  it('does not set x-trogon-atlas-branch metadata when the param is absent (baseline)', async () => {
    let receivedMeta;
    const original = fakeClient.listEntities;
    fakeClient.listEntities = (_req, meta, _opts, cb) => {
      receivedMeta = meta;
      cb(null, { entities: [], nextPageToken: '' });
    };
    const res = await request(app).get('/api/model');
    fakeClient.listEntities = original;
    expect(res.status).toBe(200);
    expect(receivedMeta?.get?.('x-trogon-atlas-branch')).toEqual([]);
  });

  it('returns 400 when ?branch= contains shell metacharacters', async () => {
    const res = await request(app).get('/api/model?branch=foo;bar');
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });

  it('returns 400 when ?branch= contains more than one slash', async () => {
    const res = await request(app).get('/api/model?branch=a/b/c');
    expect(res.status).toBe(400);
  });

  it('returns 400 when ?branch= is the reserved name "meta"', async () => {
    const res = await request(app).get('/api/model?branch=meta');
    expect(res.status).toBe(400);
  });

  it('returns 400 when ?branch= is the reserved name "meta" cased differently', async () => {
    const res = await request(app).get('/api/model?branch=META');
    expect(res.status).toBe(400);
  });

  it('returns 400 when ?branch= exceeds the max length', async () => {
    const long = 'a'.repeat(403);
    const res = await request(app).get(`/api/model?branch=${long}`);
    expect(res.status).toBe(400);
  });

  it('returns 200 when ?branch= is a valid owner-prefixed name', async () => {
    const res = await request(app).get('/api/model?branch=alex/retention-rework');
    expect(res.status).toBe(200);
  });
});

describe('GET /api/branches', () => {
  it('returns 200 with the branch list shaped from BranchInfo', async () => {
    const res = await request(app).get('/api/branches');
    expect(res.status).toBe(200);
    expect(res.body).toEqual({
      branches: [
        { name: 'alex/retention-rework', doc: 'in progress', createdAt: '2026-01-01T00:00:00Z', deltaCount: 3 },
      ],
    });
  });

  it('never forwards x-trogon-atlas-branch metadata (ListBranches is exempt from branch context)', async () => {
    let receivedMeta;
    const original = fakeClient.listBranches;
    fakeClient.listBranches = (_req, meta, _opts, cb) => {
      receivedMeta = meta;
      cb(null, { branches: [] });
    };
    const res = await request(app).get('/api/branches?branch=alex/retention-rework');
    fakeClient.listBranches = original;
    expect(res.status).toBe(200);
    expect(receivedMeta?.get?.('x-trogon-atlas-branch')).toEqual([]);
  });
});

describe('GET /api/branch-diff', () => {
  it('returns 200 with entries shaped from BranchDiffEntry, including conflictFieldPaths', async () => {
    const res = await request(app).get('/api/branch-diff?name=alex/retention-rework');
    expect(res.status).toBe(200);
    expect(res.body).toEqual({
      entries: [
        {
          ref: { kind: 'ENTITY_KIND_EVENT', id: { namespace: 'my-ns', slug: 'my-event', version: '1' } },
          status: 'STATUS_ADDED',
          base: null,
          baseEtag: '',
          ours: null,
          theirs: null,
          conflictFieldPaths: [],
        },
      ],
    });
  });

  it('returns 400 when the name query param is missing', async () => {
    const res = await request(app).get('/api/branch-diff');
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });

  it('returns 400 when the name query param is invalid', async () => {
    const res = await request(app).get('/api/branch-diff?name=foo;bar');
    expect(res.status).toBe(400);
  });

  it('returns 400 when the name query param is the reserved name "meta"', async () => {
    const res = await request(app).get('/api/branch-diff?name=meta');
    expect(res.status).toBe(400);
  });

  it('passes the requested name through to diffBranch', async () => {
    let receivedReq;
    const original = fakeClient.diffBranch;
    fakeClient.diffBranch = (req, _meta, _opts, cb) => {
      receivedReq = req;
      cb(null, { entries: [] });
    };
    const res = await request(app).get('/api/branch-diff?name=alex/retention-rework');
    fakeClient.diffBranch = original;
    expect(res.status).toBe(200);
    expect(receivedReq).toEqual({ name: 'alex/retention-rework' });
  });
});

describe('GET /api/event-model/:namespace/:slug isSafeNamespace validation', () => {
  it('returns 400 when namespace contains path traversal characters', async () => {
    const res = await request(app).get('/api/event-model/..%2Fetc%2Fpasswd/slug');
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });

  it('returns 400 when namespace contains shell metacharacters', async () => {
    const res = await request(app).get('/api/event-model/foo;bar/slug');
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });

  it('returns 400 when namespace starts with a dot', async () => {
    const res = await request(app).get('/api/event-model/.hidden/slug');
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });

  it('returns 400 when slug contains path traversal characters', async () => {
    const res = await request(app).get('/api/event-model/my-ns/..%2Fetc%2Fpasswd');
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });

  it('returns 400 when slug contains shell metacharacters', async () => {
    const res = await request(app).get('/api/event-model/my-ns/;evil');
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });

  it('returns 404 when both params are valid but entity does not exist', async () => {
    const res = await request(app).get('/api/event-model/valid-ns/valid-slug');
    expect(res.status).toBe(404);
  });
});

// ---------------------------------------------------------------------------
// Bug-hunt proofs: these assert correct behavior and are expected to FAIL
// against the current unfixed gateway. Do not "fix" production to make
// them pass while hunting; leave them red.
// ---------------------------------------------------------------------------

describe('BUG: gRPC status → HTTP mapping gaps', () => {
  it('maps DEADLINE_EXCEEDED (code 4) to HTTP 504', async () => {
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = (_req, _meta, _opts, cb) =>
      cb(Object.assign(new Error('deadline exceeded'), { code: 4 }));
    const res = await request(app).get('/api/info');
    fakeClient.getServerInfo = original;
    expect(res.status).toBe(504);
  });

  it('maps RESOURCE_EXHAUSTED (code 8) to HTTP 429', async () => {
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = (_req, _meta, _opts, cb) =>
      cb(Object.assign(new Error('resource exhausted'), { code: 8 }));
    const res = await request(app).get('/api/info');
    fakeClient.getServerInfo = original;
    expect(res.status).toBe(429);
  });

  it('maps UNIMPLEMENTED (code 12) to HTTP 501', async () => {
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = (_req, _meta, _opts, cb) =>
      cb(Object.assign(new Error('unimplemented'), { code: 12 }));
    const res = await request(app).get('/api/info');
    fakeClient.getServerInfo = original;
    expect(res.status).toBe(501);
  });
});

describe('BUG: SSE /api/changes/stream ignores ?branch=', () => {
  it('forwards x-trogon-atlas-branch metadata like GET /api/changes does', async () => {
    let receivedMeta;
    const original = fakeClient.listChanges;
    fakeClient.listChanges = (_req, meta, _opts, cb) => {
      receivedMeta = meta;
      // End the stream promptly by returning empty and letting the client
      // disconnect via timeout; we only need the first poll's metadata.
      cb(null, { events: [], nextToken: '' });
    };

    await request(app)
      .get('/api/changes/stream?branch=alex/retention-rework')
      .timeout({ deadline: 200 })
      .catch(() => {});

    fakeClient.listChanges = original;

    expect(receivedMeta?.get?.('x-trogon-atlas-branch')).toEqual([
      'alex/retention-rework',
    ]);
  });
});

describe('BUG: GET /api/validate allows missing namespace/slug', () => {
  it('returns 400 when namespace and slug query params are absent', async () => {
    const res = await request(app).get('/api/validate');
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });
});

describe('BUG: invalid TROGON_ATLAS_STUDIO_MAX_PAGES disables the pagination cap', () => {
  it('still caps pagination when MAX_PAGES env is non-numeric', async () => {
    process.env.TROGON_ATLAS_STUDIO_MAX_PAGES = 'not-a-number';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=maxpagesnan');
    const nanApp = mod.app;
    delete process.env.TROGON_ATLAS_STUDIO_MAX_PAGES;

    let calls = 0;
    const original = fakeClient.listEntities;
    fakeClient.listEntities = (_req, _meta, _opts, cb) => {
      calls += 1;
      if (calls > 60) {
        // Safety valve so a missing cap cannot hang the suite forever.
        cb(Object.assign(new Error('test safety valve: uncapped pagination'), { code: 14 }));
        return;
      }
      cb(null, {
        entities: [{ eventModel: { id: { namespace: 'n', slug: `s${calls}`, version: '1' } } }],
        // Advancing cursors so the stuck-token guard does not fire first;
        // this test isolates the MAX_PAGES ceiling itself.
        nextPageToken: `more-${calls}`,
      });
    };

    const res = await request(nanApp).get('/api/model');
    fakeClient.listEntities = original;

    // Correct behavior: hit the gateway pagination cap at the default (50)
    // and return 502. Broken behavior: page >= NaN is never true, so the
    // loop continues until the safety valve (calls > 60 → 503).
    expect(calls).toBeLessThanOrEqual(50);
    expect(res.status).toBe(502);
  });
});

describe('BUG: validate-project does not validate version query params', () => {
  it('returns 400 when projectVersion contains unsafe characters', async () => {
    const res = await request(app).get(
      '/api/validate-project?projectNamespace=my-ns&projectSlug=my-project&projectVersion=../evil',
    );
    expect(res.status).toBe(400);
    expect(res.body.error).toMatch(/projectVersion|version/i);
  });
});

describe('BUG: zero/invalid GRPC timeout disables deadlines', () => {
  it('still attaches a deadline when TROGON_ATLAS_STUDIO_GRPC_TIMEOUT_MS=0', async () => {
    process.env.TROGON_ATLAS_STUDIO_GRPC_TIMEOUT_MS = '0';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=grpctimeout0');
    const timeoutApp = mod.app;
    delete process.env.TROGON_ATLAS_STUDIO_GRPC_TIMEOUT_MS;

    let receivedOpts;
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = (_req, _meta, opts, cb) => {
      receivedOpts = opts;
      cb(null, { schemaVersion: '1', serverVersion: 'test' });
    };

    const res = await request(timeoutApp).get('/api/info');
    fakeClient.getServerInfo = original;

    expect(res.status).toBe(200);
    // Correct: a positive deadline Date must always be present so a hung
    // upstream cannot pin a gateway worker forever.
    expect(receivedOpts?.deadline).toBeInstanceOf(Date);
  });
});

describe('BUG: CORS Allow-Origin is defeated by CORP same-origin', () => {
  it('relaxes Cross-Origin-Resource-Policy when an allowlisted Origin is accepted', async () => {
    process.env.TROGON_ATLAS_STUDIO_CORS_ORIGINS = 'https://studio.example.com';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=corscorp');
    const corsApp = mod.app;
    delete process.env.TROGON_ATLAS_STUDIO_CORS_ORIGINS;

    const res = await request(corsApp)
      .get('/api/info')
      .set('Origin', 'https://studio.example.com');

    expect(res.status).toBe(200);
    expect(res.headers['access-control-allow-origin']).toBe('https://studio.example.com');
    // Browsers enforce CORP even after a CORS allow; same-origin CORP makes
    // the allowlisted cross-origin read unusable.
    expect(res.headers['cross-origin-resource-policy']).toBe('cross-origin');
  });

  it('keeps CORP same-origin when CORS is not granting this Origin', async () => {
    process.env.TROGON_ATLAS_STUDIO_CORS_ORIGINS = 'https://studio.example.com';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=corscorpdeny');
    const corsApp = mod.app;
    delete process.env.TROGON_ATLAS_STUDIO_CORS_ORIGINS;

    const res = await request(corsApp)
      .get('/api/info')
      .set('Origin', 'https://evil.example');

    expect(res.headers['access-control-allow-origin']).toBeUndefined();
    expect(res.headers['cross-origin-resource-policy']).toBe('same-origin');
  });
});

describe('BUG: /api/nats-auth credential response is cacheable', () => {
  // No arrangement hands out the WebSocket token any more, so this guards
  // the header rather than a credential: a cache that learned to store this
  // answer would serve the wrong transport to the next caller, and would
  // retain a credential again the moment one is reintroduced.
  it('sets Cache-Control: no-store on the transport answer', async () => {
    process.env.TROGON_ATLAS_AUTH_TOKEN = 'gw-secret';
    process.env.NATS_WS_AUTH_TOKEN = 'nats-secret';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=natsnocache');
    const authApp = mod.app;
    delete process.env.TROGON_ATLAS_AUTH_TOKEN;
    delete process.env.NATS_WS_AUTH_TOKEN;

    const res = await request(authApp)
      .get('/api/nats-auth')
      .set('Authorization', 'Bearer gw-secret');

    expect(res.status).toBe(200);
    expect(res.body.token).toBeNull();
    expect(String(res.headers['cache-control'] ?? '')).toMatch(/no-store/i);
  });
});

describe('BUG: SPA static fallback swallows /api and missing assets', () => {
  it('returns JSON 404 for GET /api (not the SPA shell)', async () => {
    const res = await request(app).get('/api');
    expect(res.status).toBe(404);
    expect(res.headers['content-type']).toMatch(/application\/json/);
    expect(res.body).toHaveProperty('error');
    expect(String(res.text)).not.toMatch(/<!doctype html>/i);
  });

  it('returns JSON 404 for unknown /api/* routes (not Express HTML)', async () => {
    const res = await request(app).get('/api/does-not-exist-xyz-abc');
    expect(res.status).toBe(404);
    expect(res.headers['content-type']).toMatch(/application\/json/);
    expect(res.body).toHaveProperty('error');
    expect(String(res.text)).not.toMatch(/<!DOCTYPE html>/i);
  });

  it('returns 404 for missing static assets that do not accept HTML', async () => {
    // Script/module fetches send Accept without text/html (often just */* or
    // application/javascript). Bare */* must not trigger the SPA shell.
    const res = await request(app)
      .get('/assets/definitely-missing-chunk.js')
      .set('Accept', '*/*');
    expect(res.status).toBe(404);
    expect(String(res.text)).not.toMatch(/<!doctype html>/i);
  });
});

describe('BUG: GET /api/validate does not validate version query param', () => {
  it('returns 400 when version contains unsafe characters', async () => {
    const res = await request(app).get(
      '/api/validate?namespace=my-ns&slug=my-model&version=../evil',
    );
    expect(res.status).toBe(400);
    expect(res.body.error).toMatch(/version/i);
  });
});

describe('BUG: paginateList loops on a non-advancing nextPageToken', () => {
  it('fails fast with 502 when upstream repeats the same page token', async () => {
    let calls = 0;
    const original = fakeClient.listEntities;
    fakeClient.listEntities = (_req, _meta, _opts, cb) => {
      calls += 1;
      cb(null, {
        entities: [{ eventModel: { id: { namespace: 'n', slug: `s${calls}`, version: '1' } } }],
        // Stuck cursor: same non-empty token forever.
        nextPageToken: 'stuck',
      });
    };

    const res = await request(app).get('/api/model');
    fakeClient.listEntities = original;

    // Correct: detect non-progress and abort well before MAX_PAGES (50).
    expect(calls).toBeLessThanOrEqual(2);
    expect(res.status).toBe(502);
    expect(res.body.error).toMatch(/pagination|page token|stuck/i);
  });
});

describe('GET /api/namespaces/registry', () => {
  it('reports id, owner and entity count as distinct fields', async () => {
    const original = fakeClient.listNamespaces;
    fakeClient.listNamespaces = (_req, _meta, _opts, cb) =>
      cb(null, {
        namespaces: [{ name: 'billing', id: 'ns_0192abcd', parent: 'acme', entityCount: 7 }],
      });

    const res = await request(app).get('/api/namespaces/registry');
    fakeClient.listNamespaces = original;

    expect(res.status).toBe(200);
    expect(res.body.namespaces).toEqual([
      { id: 'ns_0192abcd', name: 'billing', parent: 'acme', entityCount: 7, registered: true },
    ]);
  });

  it('marks a namespace with entities but no registry row as unregistered', async () => {
    const original = fakeClient.listNamespaces;
    fakeClient.listNamespaces = (_req, _meta, _opts, cb) =>
      cb(null, { namespaces: [{ name: 'orphan', id: '', parent: '', entityCount: 3 }] });

    const res = await request(app).get('/api/namespaces/registry');
    fakeClient.listNamespaces = original;

    // The picker endpoint maps `id || name` so every row has something to
    // filter on. Doing that here would hide the only row worth acting on:
    // the directory fails closed, so an unregistered namespace is invisible
    // to every scoped key.
    expect(res.body.namespaces[0]).toMatchObject({ id: '', name: 'orphan', registered: false });
  });

  it('does not collapse two owners who use the same name', async () => {
    const original = fakeClient.listNamespaces;
    fakeClient.listNamespaces = (_req, _meta, _opts, cb) =>
      cb(null, {
        namespaces: [
          { name: 'orders', id: 'ns_b', parent: 'beta', entityCount: 1 },
          { name: 'orders', id: 'ns_a', parent: 'acme', entityCount: 2 },
        ],
      });

    const res = await request(app).get('/api/namespaces/registry');
    fakeClient.listNamespaces = original;

    expect(res.body.namespaces).toHaveLength(2);
    expect(res.body.namespaces.map((/** @type {any} */ n) => n.parent)).toEqual(['acme', 'beta']);
  });

  it('forwards the caller authorization upstream', async () => {
    upstreamCalls.authorization = null;
    await request(app).get('/api/namespaces/registry');
    expect(upstreamCalls.authorization).not.toBeUndefined();
  });
});

describe('TROGON_ATLAS_AUTH_PASSTHROUGH', () => {
  let ptApp;
  let ptCall;

  beforeAll(async () => {
    process.env.TROGON_ATLAS_AUTH_PASSTHROUGH = 'true';
    process.env.TROGON_ATLAS_AUTH_TOKEN = 'bridge-service-token';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=passthrough');
    ptApp = mod.app;
    ptCall = mod.call;
    delete process.env.TROGON_ATLAS_AUTH_PASSTHROUGH;
    delete process.env.TROGON_ATLAS_AUTH_TOKEN;
  });

  it('rejects a request with no credential of its own', async () => {
    const res = await request(ptApp).get('/api/namespaces');
    expect(res.status).toBe(401);
  });

  it('forwards the caller token rather than the bridge token', async () => {
    upstreamCalls.authorization = null;
    const res = await request(ptApp)
      .get('/api/namespaces')
      .set('Authorization', 'Bearer alice-key');
    expect(res.status).toBe(200);
    expect(upstreamCalls.authorization).toBe('Bearer alice-key');
  });

  it('never substitutes the bridge identity for a caller that has one', async () => {
    upstreamCalls.authorization = null;
    await request(ptApp).get('/api/namespaces').set('Authorization', 'Bearer bob-key');
    // The whole point: two browsers with different keys must not both be
    // seen upstream as the bridge.
    expect(upstreamCalls.authorization).not.toBe('Bearer bridge-service-token');
    expect(upstreamCalls.authorization).toBe('Bearer bob-key');
  });

  it('refuses a mutation even from a caller holding a credential the server would accept', async () => {
    upstreamCalls.registerNamespace = null;
    const res = await request(ptApp)
      .post('/api/namespaces')
      .set('Authorization', 'Bearer alice-key')
      .send({ name: 'billing' });
    expect(res.status).toBe(403);
    expect(upstreamCalls.registerNamespace).toBeNull();
  });

  it('does not accept the bridge token as a caller credential without upstream say-so', async () => {
    // It still reaches upstream (the bridge is not the authority), but it is
    // forwarded as presented, not privileged locally into a bypass.
    upstreamCalls.authorization = null;
    const res = await request(ptApp)
      .get('/api/namespaces')
      .set('Authorization', 'Bearer bridge-service-token');
    expect(res.status).toBe(200);
    expect(upstreamCalls.authorization).toBe('Bearer bridge-service-token');
  });

  it('answers /api/info with the bridge identity, the one route with no caller token to forward', async () => {
    const original = fakeClient.getServerInfo;
    let forwarded = null;
    fakeClient.getServerInfo = (_req, meta, _opts, cb) => {
      forwarded = meta?.get('authorization')?.[0] ?? null;
      cb(null, { schemaVersion: '1', serverVersion: 'test' });
    };
    const res = await request(ptApp).get('/api/info');
    fakeClient.getServerInfo = original;
    expect(res.status).toBe(200);
    expect(forwarded).toBe('Bearer bridge-service-token');
  });

  it('fails closed rather than borrowing the bridge identity when a caller token is missing', async () => {
    // Every real route goes through `globalApiAuth`, which already 401s a
    // request with no token before this is reachable. This exercises the
    // fallback itself, so a future call site that forgets to run inside the
    // request's context fails loudly instead of quietly calling upstream as
    // the bridge.
    upstreamCalls.authorization = null;
    await expect(ptCall('listNamespaces', {})).rejects.toMatchObject({ status: 401 });
    expect(upstreamCalls.authorization).toBeNull();
  });
});

describe('TROGON_ATLAS_AUTH_PASSTHROUGH does not turn /api/nats-auth into an open credential dispenser', () => {
  let ptApp;

  beforeAll(async () => {
    process.env.TROGON_ATLAS_AUTH_PASSTHROUGH = 'true';
    process.env.NATS_WS_AUTH_TOKEN = 'nats-ws-secret';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=passthroughnats');
    ptApp = mod.app;
    delete process.env.TROGON_ATLAS_AUTH_PASSTHROUGH;
    delete process.env.NATS_WS_AUTH_TOKEN;
  });

  it('withholds the NATS credential when the server rejects the caller', async () => {
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = (_req, _meta, _opts, cb) =>
      cb(Object.assign(new Error('unauthenticated'), { code: 16 }));
    const res = await request(ptApp).get('/api/nats-auth').set('Authorization', 'Bearer forged');
    fakeClient.getServerInfo = original;
    expect(res.status).toBe(401);
    expect(JSON.stringify(res.body)).not.toContain('nats-ws-secret');
  });

  it('answers 403 when the server knows the caller but denies it', async () => {
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = (_req, _meta, _opts, cb) =>
      cb(Object.assign(new Error('denied'), { code: 7 }));
    const res = await request(ptApp).get('/api/nats-auth').set('Authorization', 'Bearer reader');
    fakeClient.getServerInfo = original;
    expect(res.status).toBe(403);
    expect(JSON.stringify(res.body)).not.toContain('nats-ws-secret');
  });

  it('withholds the NATS credential even from a caller the server vouches for', async () => {
    // The listener grants one unscoped read of the entity bucket to every
    // client it admits, so a per-principal deployment must not use it at all.
    // Being a real caller does not change that; it changes only which
    // transport the answer names.
    const res = await request(ptApp).get('/api/nats-auth').set('Authorization', 'Bearer alice-key');
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ token: null, transport: 'sse' });
    expect(JSON.stringify(res.body)).not.toContain('nats-ws-secret');
  });

  it('refuses the change stream before the headers go out', async () => {
    // Once the stream is open the status is spent, and an unusable
    // credential would reach the browser as a 200 that emits nothing
    // forever instead of something it can prompt about.
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = (_req, _meta, _opts, cb) =>
      cb(Object.assign(new Error('unauthenticated'), { code: 16 }));
    const res = await request(ptApp)
      .get('/api/changes/stream')
      .set('Authorization', 'Bearer forged');
    fakeClient.getServerInfo = original;
    expect(res.status).toBe(401);
    expect(res.headers['content-type']).not.toContain('text/event-stream');
  });

  it('answers 403 on the change stream when the server denies a known caller', async () => {
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = (_req, _meta, _opts, cb) =>
      cb(Object.assign(new Error('denied'), { code: 7 }));
    const res = await request(ptApp)
      .get('/api/changes/stream')
      .set('Authorization', 'Bearer reader');
    fakeClient.getServerInfo = original;
    expect(res.status).toBe(403);
    expect(res.headers['content-type']).not.toContain('text/event-stream');
  });
});

describe('/api/nats-auth names the transport the deployment can actually authorize', () => {
  it('offers NATS when nothing on the deployment is scoped', async () => {
    process.env.NATS_WS_AUTH_TOKEN = '';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=transportnats');
    const res = await request(mod.app).get('/api/nats-auth');
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ token: null, transport: 'nats' });
  });

  it('refuses NATS to a bridge carrying a key, whose scope it cannot measure', async () => {
    // The bucket read the WebSocket grants is the whole bucket. A bridge
    // holding one tenant's key would be routing browsers past that key into
    // every other tenant's entities, which is the leak the key was for.
    process.env.NATS_WS_AUTH_TOKEN = 'nats-ws-secret';
    process.env.TROGON_ATLAS_AUTH_TOKEN = 'bridge-token';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=transportfixed');
    delete process.env.NATS_WS_AUTH_TOKEN;
    delete process.env.TROGON_ATLAS_AUTH_TOKEN;
    const res = await request(mod.app)
      .get('/api/nats-auth')
      .set('Authorization', 'Bearer bridge-token');
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ token: null, transport: 'sse' });
  });

  it('offers SSE when the bridge has no identity of its own', async () => {
    process.env.TROGON_ATLAS_AUTH_PASSTHROUGH = 'true';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=transportsse');
    delete process.env.TROGON_ATLAS_AUTH_PASSTHROUGH;
    const res = await request(mod.app).get('/api/nats-auth').set('Authorization', 'Bearer alice-key');
    expect(res.status).toBe(200);
    expect(res.body.transport).toBe('sse');
    expect(res.body.token).toBeNull();
  });
});

describe('Studio is an observation surface', () => {
  let authApp;

  beforeAll(async () => {
    process.env.TROGON_ATLAS_AUTH_TOKEN = 'observer-secret';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=observer');
    authApp = mod.app;
    delete process.env.TROGON_ATLAS_AUTH_TOKEN;
  });

  const resetUpstream = () => {
    upstreamCalls.registerNamespace = null;
    upstreamCalls.moveNamespace = null;
    upstreamCalls.deleteByQuery = null;
    upstreamCalls.putEntity = null;
  };

  it('refuses bulk deletion sent directly with the studio credential', async () => {
    resetUpstream();
    const res = await request(authApp)
      .post('/api/delete-by-query')
      .set('Authorization', 'Bearer observer-secret')
      .send({ kind: 'event', namespace: 'acme', max_deletes: 10 });
    expect(res.status).toBe(403);
    expect(res.body).toHaveProperty('error');
    expect(upstreamCalls.deleteByQuery).toBeNull();
  });

  it('refuses namespace registration sent directly with the studio credential', async () => {
    resetUpstream();
    const res = await request(authApp)
      .post('/api/namespaces')
      .set('Authorization', 'Bearer observer-secret')
      .send({ name: 'billing', parent: 'acme' });
    expect(res.status).toBe(403);
    expect(upstreamCalls.registerNamespace).toBeNull();
  });

  it('refuses an ownership change sent directly with the studio credential', async () => {
    resetUpstream();
    const res = await request(authApp)
      .post('/api/namespaces/move')
      .set('Authorization', 'Bearer observer-secret')
      .send({ id: 'ns_0192abcd', parent: 'beta' });
    expect(res.status).toBe(403);
    expect(upstreamCalls.moveNamespace).toBeNull();
  });

  it.each(['post', 'put', 'patch', 'delete'])('refuses %s on any api path, including ones that do not exist yet', async (method) => {
    const res = await request(authApp)[method]('/api/entities/acme/order-placed')
      .set('Authorization', 'Bearer observer-secret')
      .send({});
    expect(res.status).toBe(403);
    expect(res.headers['content-type']).toMatch(/application\/json/);
  });

  it('refuses a mutation before reading its body, so a malformed one gets the same answer', async () => {
    const res = await request(authApp)
      .post('/api/delete-by-query')
      .set('Authorization', 'Bearer observer-secret')
      .set('Content-Type', 'application/json')
      .send('{not-json');
    expect(res.status).toBe(403);
    expect(String(res.text)).not.toMatch(/<!DOCTYPE html>/i);
  });

  it('refuses a mutation without a credential too', async () => {
    const res = await request(authApp).post('/api/namespaces').send({ name: 'billing' });
    expect(res.status).toBe(403);
  });

  it('keeps read access for the same credential', async () => {
    const res = await request(authApp)
      .get('/api/namespaces/registry')
      .set('Authorization', 'Bearer observer-secret');
    expect(res.status).toBe(200);
  });

  it('never forwards an RPC outside the read allowlist upstream', async () => {
    resetUpstream();
    await expect(call('putEntity', { entity: {} })).rejects.toMatchObject({ status: 403 });
    await expect(call('deleteByQuery', { namespace: 'acme', maxDeletes: 1 })).rejects.toMatchObject({ status: 403 });
    await expect(call('registerNamespace', { name: 'billing' })).rejects.toMatchObject({ status: 403 });
    await expect(call('moveNamespace', { id: 'ns_0192abcd', parent: 'beta' })).rejects.toMatchObject({ status: 403 });
    expect(upstreamCalls.putEntity).toBeNull();
    expect(upstreamCalls.deleteByQuery).toBeNull();
    expect(upstreamCalls.registerNamespace).toBeNull();
    expect(upstreamCalls.moveNamespace).toBeNull();
  });

  it('still forwards read RPCs', async () => {
    await expect(call('getServerInfo', {})).resolves.toMatchObject({ serverVersion: 'test' });
  });
});

describe('Type library routes', () => {
  const library = {
    id: { namespace: 'acme', slug: 'acme.v1', version: '1' },
    files: [{ path: 'acme/v1/a.proto', content: 'syntax = "proto3";' }],
    dependencies: [],
  };

  it('compiles a library without any other write route opening up', async () => {
    upstreamCalls.compileTypeLibrary = null;
    const res = await request(app)
      .post('/api/type-libraries/compile?branch=alex/retention-rework')
      .send({ library: { ...library, title: 'ignored', provenance: { inline: {} } } });
    expect(res.status).toBe(200);
    expect(res.body).toEqual({
      diagnostics: [{ path: 'acme/v1/a.proto', line: 3, column: 7, message: 'syntax error' }],
      compatibilityViolations: [],
    });
    expect(upstreamCalls.compileTypeLibrary).toEqual({ req: { library }, branch: 'alex/retention-rework' });
    const other = await request(app).post('/api/type-libraries/messages').send({});
    expect(other.status).toBe(403);
  });

  it('rejects a malformed library before it reaches the server', async () => {
    upstreamCalls.compileTypeLibrary = null;
    for (const body of [
      {},
      { library: { ...library, id: { namespace: '../x', slug: 'acme.v1' } } },
      { library: { ...library, files: [{ path: 1 }] } },
      { library: { ...library, dependencies: [{ id: { namespace: 'acme', slug: 'b', version: 'x' } }] } },
    ]) {
      const res = await request(app).post('/api/type-libraries/compile').send(body);
      expect(res.status).toBe(400);
    }
    expect(upstreamCalls.compileTypeLibrary).toBeNull();
  });

  it('lists the message names a namespace declares, without the descriptor bytes', async () => {
    const res = await request(app).get('/api/type-libraries/messages?namespace=acme');
    expect(res.status).toBe(200);
    expect(res.body).toEqual({
      messages: [{ fullName: 'acme.v1.Money', library: { namespace: 'acme', slug: 'acme.v1', version: '1' } }],
    });
    expect(upstreamCalls.getTypeLibraryDescriptorSet).toEqual({ namespace: 'acme' });
  });

  it('requires exactly one safe namespace to list messages', async () => {
    for (const q of ['', '?namespace=a,b', '?namespace=..']) {
      const res = await request(app).get(`/api/type-libraries/messages${q}`);
      expect(res.status).toBe(400);
    }
  });
});

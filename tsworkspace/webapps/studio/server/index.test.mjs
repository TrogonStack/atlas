import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import request from 'supertest';

const here = path.dirname(fileURLToPath(import.meta.url));
const distDir = path.resolve(here, '..', 'dist');

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
  exportEventModelManifests: null,
  authorization: null,
};

const ACME_CHANGESET = {
  id: '0192f000-0000-7000-8000-000000000003',
  author: 'acme-agent',
  at: '2026-01-03T00:00:00Z',
  message: 'put event acme/order.placed',
  branch: '',
  rpc: 'PutEntity',
  operationId: '',
  ops: [{ kind: 'KIND_PUT', entity: { kind: 'ENTITY_KIND_EVENT', id: { namespace: 'acme', slug: 'order.placed', version: '1' } } }],
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
  exportEventModelManifests: (req, _meta, _opts, cb) => {
    upstreamCalls.exportEventModelManifests = req;
    cb(null, { manifests: 'kind: EventModel\n', externalManifests: '' });
  },
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
  getEntityHistory: (_req, _meta, _opts, cb) =>
    cb(null, {
      revisions: [
        {
          changesetId: '0192f000-0000-7000-8000-000000000002',
          author: 'acme-agent',
          at: '2026-01-02T00:00:00Z',
          rpc: 'PutEntity',
          branch: '',
          entity: { kind: 'ENTITY_KIND_EVENT', id: { namespace: 'acme', slug: 'order.placed', version: '1' } },
          kind: 'KIND_PUT',
          before: { event: { title: 'Order placed' } },
          after: { event: { title: 'Order placed by shopper' } },
        },
      ],
      nextPageToken: '0192f000-0000-7000-8000-000000000002',
    }),
  listChangesets: (_req, _meta, _opts, cb) =>
    cb(null, {
      changesets: [ACME_CHANGESET],
      nextPageToken: '0192f000-0000-7000-8000-000000000003',
    }),
  getEventModelProjection: (_req, _meta, _opts, cb) => cb(null, { projection: { entities: {} } }),
  getChangeset: (req, _meta, _opts, cb) =>
    req.id === ACME_CHANGESET.id
      ? cb(null, { changeset: ACME_CHANGESET })
      : cb(Object.assign(new Error('entity not found'), { code: 5 })),
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

vi.mock('protobufjs', async (importOriginal) => {
  const actual = (await importOriginal()).default;
  const nativePath = await import('node:path');
  const nativeUrl = await import('node:url');
  const protoRoot = nativePath.resolve(nativePath.dirname(nativeUrl.fileURLToPath(import.meta.url)), '../../../../proto');
  const responsibilityRoot = new actual.Root();
  responsibilityRoot.resolvePath = (_origin, target) => nativePath.resolve(protoRoot, target);
  responsibilityRoot.loadSync(nativePath.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'));
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
    lookupType: (name) => {
      if (/\.(GetSource|ListSource)/.test(name)) return responsibilityRoot.lookupType(name);
      if (/\.(ListSubjectAssignments|ListAssigneeResponsibilities|ListTeamMembers|ListPersonTeams|GetTeamContacts|ListSubjectDecisions|ListSubjectDocuments|ListDecisionAmendments|GetDecisionDetail|GetDocumentDetail|ListIncomingDecisionDeclarations|ListIncomingDocumentDeclarations|GetDesignGraphDetail|ListSubjectDesignGraphs|TraverseDesignGraphTopology|ListIncomingDesignGraphSubflows|InspectHumanInteraction)Request$/.test(name)) return responsibilityRoot.lookupType(name);
      return { decode: () => ({}), toObject: () => ({}) };
    },
    lookupEnum: (name) => {
      if (name === 'trogonatlas.eventmodel.v1alpha1.EntityKind') {
        return { values: fakeEntityKindValues };
      }
      throw new Error(`enum not found: ${name}`);
    },
  };
  return {
    default: {
      ...actual,
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

  it('allows /api/info without a token', async () => {
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

  describe('namespace scope', () => {
    let seen;
    let original;

    beforeEach(() => {
      original = fakeClient.validateProject;
      seen = undefined;
      fakeClient.validateProject = (req, _meta, _opts, cb) => {
        seen = req;
        cb(null, { reports: [], totalErrors: 0, totalWarnings: 0, totalInfo: 0 });
      };
    });

    afterEach(() => {
      fakeClient.validateProject = original;
    });

    it('maps ?namespace= to the namespace scope', async () => {
      const res = await request(app).get('/api/validate-project?namespace=accounts');
      expect(res.status).toBe(200);
      expect(seen).toEqual({ namespace: 'accounts' });
    });

    it('still maps project and domain scopes to their ids', async () => {
      await request(app).get('/api/validate-project?projectNamespace=acme&projectSlug=acme');
      expect(seen).toEqual({ projectId: { namespace: 'acme', slug: 'acme', version: '1' } });
      await request(app).get('/api/validate-project?domainNamespace=shop&domainSlug=shop&domainVersion=2');
      expect(seen).toEqual({ domainId: { namespace: 'shop', slug: 'shop', version: '2' } });
    });

    it('returns 400 when the namespace is unsafe or too long', async () => {
      expect((await request(app).get('/api/validate-project?namespace=../bad')).status).toBe(400);
      expect((await request(app).get(`/api/validate-project?namespace=${'a'.repeat(5000)}`)).status).toBe(400);
      expect(seen).toBeUndefined();
    });

    it('returns 400 unless exactly one scope is set', async () => {
      const both = await request(app).get(
        '/api/validate-project?namespace=accounts&projectNamespace=acme&projectSlug=acme',
      );
      expect(both.status).toBe(400);
      expect(both.body.error).toMatch(/exactly one/);
      expect(seen).toBeUndefined();
      const none = await request(app).get('/api/validate-project');
      expect(none.status).toBe(400);
      expect(none.body.error).toMatch(/exactly one/);
    });
  });
});

describe('GET /api/event-model/:namespace/:slug/manifests', () => {
  it('returns the board manifests for the model', async () => {
    const res = await request(app).get('/api/event-model/my-ns/my-model/manifests');
    expect(res.status).toBe(200);
    expect(res.body.manifests).toBe('kind: EventModel\n');
    expect(upstreamCalls.exportEventModelManifests).toEqual({
      id: { namespace: 'my-ns', slug: 'my-model', version: '1' },
    });
  });

  it('returns 400 for an unsafe namespace', async () => {
    const res = await request(app).get('/api/event-model/bad%20ns/my-model/manifests');
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

  it('loads the domains and subdomains declared in the namespace, and the project a domain names', async () => {
    const originalGetEntity = fakeClient.getEntity;
    const originalListEntities = fakeClient.listEntities;
    const listed = [];
    fakeClient.getEntity = (req, _meta, _opts, cb) => {
      const { kind, id } = req;
      if (kind === 12) {
        cb(null, { entity: { eventModel: { id, title: 'Stray model', members: [] } } });
        return;
      }
      if (kind === 16) {
        cb(null, {
          entity: {
            boundedContext: {
              id: { namespace: 'my-ns', slug: 'my-ns', version: '1' },
              title: 'My context',
              realizes: [{ id: { namespace: 'problem', slug: 'billing', version: '1' } }],
            },
          },
        });
        return;
      }
      if (kind === 18 && id.slug === 'billing') {
        cb(null, {
          entity: {
            subdomain: {
              id,
              title: 'Billing',
              domain: { id: { namespace: 'problem', slug: 'problem', version: '1' } },
            },
          },
        });
        return;
      }
      if (kind === 17 && id.namespace === 'problem') {
        cb(null, { entity: { domain: { id, title: 'Problem' } } });
        return;
      }
      if (kind === 20) {
        cb(null, { entity: { project: { id, title: 'Acme' } } });
        return;
      }
      cb(null, { entity: null });
    };
    fakeClient.listEntities = (req, _meta, _opts, cb) => {
      listed.push(req);
      const strayDomain = {
        domain: {
          id: { namespace: 'my-ns', slug: 'my-ns', version: '1' },
          title: 'Stray domain',
          project: { id: { namespace: 'acme', slug: 'acme', version: '1' } },
        },
      };
      const parent = {
        domain: { id: { namespace: 'problem', slug: 'problem', version: '2' }, title: 'Problem again' },
      };
      cb(null, { entities: req.namespaces?.[0] === 'my-ns' ? [strayDomain, parent] : [], nextPageToken: '' });
    };

    const res = await request(app).get('/api/event-model/my-ns/stray-model');

    fakeClient.getEntity = originalGetEntity;
    fakeClient.listEntities = originalListEntities;

    expect(res.status).toBe(200);
    expect(listed.some((r) => r.namespaces?.[0] === 'my-ns' && r.kinds?.join() === '17,18')).toBe(true);
    const domains = res.body.entities.filter((e) => e.domain).map((e) => e.domain.title);
    expect(domains.sort()).toEqual(['Problem', 'Stray domain']);
    expect(res.body.entities.some((e) => e.project?.id?.slug === 'acme')).toBe(true);
  });

  it('keeps the rest of the knowledge graph when listing the namespace fails', async () => {
    const originalGetEntity = fakeClient.getEntity;
    const originalListEntities = fakeClient.listEntities;
    fakeClient.getEntity = (req, _meta, _opts, cb) => {
      const { kind, id } = req;
      if (kind === 12) {
        cb(null, { entity: { eventModel: { id, title: 'Model', members: [] } } });
        return;
      }
      if (kind === 16) {
        cb(null, { entity: { boundedContext: { id, title: 'Context' } } });
        return;
      }
      cb(null, { entity: null });
    };
    fakeClient.listEntities = (_req, _meta, _opts, cb) => cb(new Error('listing exploded'));

    const res = await request(app).get('/api/event-model/my-ns/list-fails');

    fakeClient.getEntity = originalGetEntity;
    fakeClient.listEntities = originalListEntities;

    expect(res.status).toBe(200);
    expect(res.body.entities.map((e) => Object.keys(e)[0]).sort()).toEqual(['boundedContext', 'eventModel']);
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

describe('GET /api/by-domain query controls', () => {
  let invoke;

  beforeEach(() => {
    invoke = vi.spyOn(fakeClient, 'listEntitiesByDomain').mockImplementation((_req, _meta, _opts, cb) => {
      cb(null, { entities: [], nextPageToken: 'next+/= token' });
    });
  });

  afterEach(() => {
    invoke.mockRestore();
  });

  it('preserves omitted defaults without adding page controls', async () => {
    const res = await request(app).get('/api/by-domain/my-ns/checkout');
    expect(res.status).toBe(200);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke.mock.calls[0][0]).toEqual({
      domainId: { namespace: 'my-ns', slug: 'checkout', version: '1' },
      kinds: [],
      latestVersionsOnly: true,
    });
    expect(res.body).toEqual({ entities: [], nextPageToken: 'next+/= token' });
  });

  it.each(['9007199254740993', '18446744073709551615'])(
    'forwards exact version %s, false, page controls, kinds and branch metadata', async (version) => {
      const res = await request(app).get('/api/by-domain/my-ns/checkout').query({
        version, latestVersionsOnly: 'false', pageSize: '2147483647',
        pageToken: 'opaque+/= token&value', kinds: 'read-model,commandslice,ENTITY_KIND_EVENT',
        branch: 'review/domain',
      });
      expect(res.status).toBe(200);
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(invoke.mock.calls[0][0]).toEqual({
        domainId: { namespace: 'my-ns', slug: 'checkout', version },
        kinds: [3, 8, 1], latestVersionsOnly: false,
        pageSize: 2147483647, pageToken: 'opaque+/= token&value',
      });
      expect(invoke.mock.calls[0][1].get('x-trogon-atlas-branch')).toEqual(['review/domain']);
      expect(res.body).toEqual({ entities: [], nextPageToken: 'next+/= token' });
    },
  );

  it('accepts explicit true, minimum numbers, an empty cursor and the branch header', async () => {
    const res = await request(app).get('/api/by-domain/my-ns/checkout')
      .set('x-trogon-atlas-branch', 'review/domain')
      .query({ version: '1', latestVersionsOnly: 'true', pageSize: '1', pageToken: '' });
    expect(res.status).toBe(200);
    expect(invoke.mock.calls[0][0]).toEqual({
      domainId: { namespace: 'my-ns', slug: 'checkout', version: '1' },
      kinds: [], latestVersionsOnly: true, pageSize: 1, pageToken: '',
    });
    expect(invoke.mock.calls[0][1].get('x-trogon-atlas-branch')).toEqual(['review/domain']);
  });

  it.each([
    ...['', '0', '-1', '1.5', '1e2', '+1', ' 1', '01', 'Infinity', '18446744073709551616', '999999999999999999999']
      .map((value) => `version=${encodeURIComponent(value)}`),
    ...['', '0', '-1', '1.5', '1e2', '+1', ' 1', '01', 'Infinity', '2147483648']
      .map((value) => `pageSize=${encodeURIComponent(value)}`),
    ...['', 'TRUE', 'False', '1', '0', 'yes', ' false']
      .map((value) => `latestVersionsOnly=${encodeURIComponent(value)}`),
    ...['version', 'latestVersionsOnly', 'pageSize', 'pageToken'].flatMap((key) => [
      `${key}=1&${key}=1`, `${key}[]=1`, `${key}[value]=1`,
    ]),
  ])('rejects invalid query %s before RPC', async (query) => {
    const res = await request(app).get(`/api/by-domain/my-ns/checkout?${query}`);
    expect(res.status).toBe(400);
    expect(res.body.error).toEqual(expect.any(String));
    expect(invoke).not.toHaveBeenCalled();
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

describe('GET /api/entity-history', () => {
  it('returns revisions newest first with before/after and the next page token', async () => {
    const res = await request(app).get('/api/entity-history?kind=event&namespace=acme&slug=order.placed');
    expect(res.status).toBe(200);
    expect(res.body.nextPageToken).toBe('0192f000-0000-7000-8000-000000000002');
    expect(res.body.revisions).toHaveLength(1);
    expect(res.body.revisions[0]).toMatchObject({
      changesetId: '0192f000-0000-7000-8000-000000000002',
      author: 'acme-agent',
      kind: 'KIND_PUT',
      before: { event: { title: 'Order placed' } },
      after: { event: { title: 'Order placed by shopper' } },
    });
  });

  it('forwards the ref, page token and branch to getEntityHistory', async () => {
    let received;
    const original = fakeClient.getEntityHistory;
    fakeClient.getEntityHistory = (req, meta, _opts, cb) => {
      received = { req, branch: meta.get('x-trogon-atlas-branch') };
      cb(null, { revisions: [] });
    };
    const res = await request(app).get(
      '/api/entity-history?kind=readModel&namespace=acme&slug=orders&version=2&pageToken=0192f000-0000-7000-8000-000000000001&branch=alex/retention-rework',
    );
    fakeClient.getEntityHistory = original;
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ revisions: [], nextPageToken: '' });
    expect(received.req).toEqual({
      ref: { kind: 3, id: { namespace: 'acme', slug: 'orders', version: '2' } },
      pageToken: '0192f000-0000-7000-8000-000000000001',
    });
    expect(received.branch).toEqual(['alex/retention-rework']);
  });

  it.each([
    ['slug is missing', '/api/entity-history?kind=event&namespace=acme'],
    ['namespace is unsafe', '/api/entity-history?kind=event&namespace=..&slug=order.placed'],
    ['kind is unknown', '/api/entity-history?kind=widget&namespace=acme&slug=order.placed'],
    ['pageToken is unsafe', '/api/entity-history?kind=event&namespace=acme&slug=order.placed&pageToken=a%2Fb'],
  ])('returns 400 when %s', async (_why, url) => {
    const res = await request(app).get(url);
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });
});

describe('GET /api/changesets', () => {
  it('returns changesets newest first with their ops and the next page token', async () => {
    const res = await request(app).get('/api/changesets?namespace=acme');
    expect(res.status).toBe(200);
    expect(res.body).toEqual({
      changesets: [
        {
          id: '0192f000-0000-7000-8000-000000000003',
          author: 'acme-agent',
          at: '2026-01-03T00:00:00Z',
          message: 'put event acme/order.placed',
          branch: '',
          rpc: 'PutEntity',
          ops: [
            { kind: 'KIND_PUT', entity: { kind: 'ENTITY_KIND_EVENT', namespace: 'acme', slug: 'order.placed', version: '1' } },
          ],
        },
      ],
      nextPageToken: '0192f000-0000-7000-8000-000000000003',
    });
  });

  it('forwards namespace, page token, page size and the previewed branch as the branch filter', async () => {
    let received;
    const original = fakeClient.listChangesets;
    fakeClient.listChangesets = (req, _meta, _opts, cb) => {
      received = req;
      cb(null, {});
    };
    const res = await request(app).get(
      '/api/changesets?namespace=acme&pageToken=0192f000-0000-7000-8000-000000000001&pageSize=25&branch=alex/retention-rework',
    );
    fakeClient.listChangesets = original;
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ changesets: [], nextPageToken: '' });
    expect(received).toEqual({
      namespace: 'acme',
      pageToken: '0192f000-0000-7000-8000-000000000001',
      pageSize: 25,
      branch: 'alex/retention-rework',
    });
  });

  it('sends no branch filter on baseline, so branch changesets come back tagged with their branch', async () => {
    let received;
    const original = fakeClient.listChangesets;
    fakeClient.listChangesets = (req, _meta, _opts, cb) => {
      received = req;
      cb(null, { changesets: [] });
    };
    const res = await request(app).get('/api/changesets?namespace=acme');
    fakeClient.listChangesets = original;
    expect(res.status).toBe(200);
    expect(received).toEqual({ namespace: 'acme' });
  });

  it.each([
    ['namespace is missing', '/api/changesets'],
    ['namespace is unsafe', '/api/changesets?namespace=..'],
    ['pageToken is not a changeset id', '/api/changesets?namespace=acme&pageToken=a%2Fb'],
    ['pageToken is uppercase', '/api/changesets?namespace=acme&pageToken=0192F000-0000-7000-8000-000000000001'],
    ['pageSize is not a positive integer', '/api/changesets?namespace=acme&pageSize=-1'],
    ['branch is unsafe', '/api/changesets?namespace=acme&branch=..%2Fx'],
  ])('returns 400 when %s', async (_why, url) => {
    const res = await request(app).get(url);
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });
});

describe('GET /api/changesets/:id', () => {
  it('returns one changeset in the list shape', async () => {
    const res = await request(app).get('/api/changesets/0192f000-0000-7000-8000-000000000003');
    expect(res.status).toBe(200);
    expect(res.body.changeset).toMatchObject({
      id: '0192f000-0000-7000-8000-000000000003',
      rpc: 'PutEntity',
      ops: [{ kind: 'KIND_PUT', entity: { kind: 'ENTITY_KIND_EVENT', namespace: 'acme', slug: 'order.placed' } }],
    });
  });

  it('maps an unknown id to 404', async () => {
    const res = await request(app).get('/api/changesets/0192f000-0000-7000-8000-0000000000ff');
    expect(res.status).toBe(404);
  });

  it('returns 400 for an id that is not a lowercase hyphenated UUID', async () => {
    const res = await request(app).get('/api/changesets/not-a-uuid');
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
  });
});

describe('GET /api/event-model/:namespace/:slug?asOf=', () => {
  const AS_OF = '0192f000-0000-7000-8000-000000000003';
  const placed = { namespace: 'acme', slug: 'order.placed', version: '1' };
  const summary = { namespace: 'acme', slug: 'order.summary', version: '1' };
  const shipped = { namespace: 'shipping', slug: 'parcel.shipped', version: '1' };
  const projection = {
    projection: {
      eventModel: {
        id: { namespace: 'acme', slug: 'checkout', version: '1' },
        title: 'Checkout',
        members: [
          { kind: 'ENTITY_KIND_EVENT', id: placed },
          { kind: 'ENTITY_KIND_READ_MODEL', id: summary },
        ],
      },
      entities: {
        'event:acme/order.placed@1': { event: { id: placed, title: 'Order placed' } },
        'read_model:acme/order.summary@1': { readModel: { id: summary, sourceEvents: [{ id: shipped }] } },
        'event:shipping/parcel.shipped@1': { event: { id: shipped, title: 'Parcel shipped' } },
        // Reached from a member, but not a member: the live route leaves it out too.
        'schema:acme/order-placed-payload@1': { schema: { id: { namespace: 'acme', slug: 'order-placed-payload', version: '1' } } },
      },
    },
    truncated: false,
  };

  /** @param {(req: any, meta: any, cb: Function) => void} impl */
  async function withProjection(impl, run) {
    const original = fakeClient.getEventModelProjection;
    const originalGetEntity = fakeClient.getEntity;
    fakeClient.getEventModelProjection = (req, meta, _opts, cb) => impl(req, meta, cb);
    fakeClient.getEntity = () => {
      throw new Error('an as-of read must not fall back to live getEntity');
    };
    try {
      return await run();
    } finally {
      fakeClient.getEventModelProjection = original;
      fakeClient.getEntity = originalGetEntity;
    }
  }

  it('forwards asOfChangeset to getEventModelProjection on baseline and returns the members and halo', async () => {
    let received;
    let branchHeader;
    const res = await withProjection(
      (req, meta, cb) => {
        received = req;
        branchHeader = meta.get('x-trogon-atlas-branch');
        cb(null, projection);
      },
      () => request(app).get(`/api/event-model/acme/checkout?asOf=${AS_OF}`),
    );
    expect(res.status).toBe(200);
    expect(received).toEqual({
      id: { namespace: 'acme', slug: 'checkout', version: '1' },
      includeCrossModel: true,
      asOfChangeset: AS_OF,
    });
    expect(branchHeader).toEqual([]);
    expect(res.body.entities).toEqual([
      { eventModel: projection.projection.eventModel },
      { event: { id: placed, title: 'Order placed' } },
      { readModel: { id: summary, sourceEvents: [{ id: shipped }] } },
      { event: { id: shipped, title: 'Parcel shipped' } },
    ]);
  });

  it('flags a truncated closure that dropped a halo event', async () => {
    const { 'event:shipping/parcel.shipped@1': _dropped, ...kept } = projection.projection.entities;
    const res = await withProjection(
      (_req, _meta, cb) => cb(null, { projection: { ...projection.projection, entities: kept }, truncated: true }),
      () => request(app).get(`/api/event-model/acme/checkout?asOf=${AS_OF}`),
    );
    expect(res.status).toBe(200);
    expect(res.body.incomplete).toBe(true);
  });

  it('does not flag a truncated closure that kept every member and halo event', async () => {
    const res = await withProjection(
      (_req, _meta, cb) => cb(null, { ...projection, truncated: true }),
      () => request(app).get(`/api/event-model/acme/checkout?asOf=${AS_OF}`),
    );
    expect(res.status).toBe(200);
    expect(res.body.incomplete).toBeUndefined();
  });

  it('maps an unknown changeset to 404', async () => {
    const res = await withProjection(
      (_req, _meta, cb) => cb(Object.assign(new Error('entity not found'), { code: 5 })),
      () => request(app).get(`/api/event-model/acme/checkout?asOf=${AS_OF}`),
    );
    expect(res.status).toBe(404);
  });

  it('returns 404 when the model did not exist yet at that changeset', async () => {
    const res = await withProjection(
      (_req, _meta, cb) => cb(null, { projection: { entities: {} } }),
      () => request(app).get(`/api/event-model/acme/checkout?asOf=${AS_OF}`),
    );
    expect(res.status).toBe(404);
  });

  it.each([
    ['asOf is not a UUID', '/api/event-model/acme/checkout?asOf=yesterday'],
    ['asOf is uppercase', '/api/event-model/acme/checkout?asOf=0192F000-0000-7000-8000-000000000003'],
    ['asOf comes with ?branch=', `/api/event-model/acme/checkout?asOf=${AS_OF}&branch=alex/retention-rework`],
  ])('returns 400 when %s', async (_why, url) => {
    let called = false;
    const res = await withProjection(
      (_req, _meta, cb) => {
        called = true;
        cb(null, projection);
      },
      () => request(app).get(url),
    );
    expect(res.status).toBe(400);
    expect(res.body).toHaveProperty('error');
    expect(called).toBe(false);
  });

  it('returns 400 when asOf comes with the branch header', async () => {
    const res = await withProjection(
      (_req, _meta, cb) => cb(null, projection),
      () =>
        request(app)
          .get(`/api/event-model/acme/checkout?asOf=${AS_OF}`)
          .set('x-trogon-atlas-branch', 'alex/retention-rework'),
    );
    expect(res.status).toBe(400);
    expect(res.body.error).toMatch(/baseline/);
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

describe('SPA/static fallback rate limiter', () => {
  let createdDist = false;

  beforeAll(() => {
    if (fs.existsSync(distDir)) return;
    createdDist = true;
    fs.mkdirSync(distDir, { recursive: true });
    fs.writeFileSync(path.join(distDir, 'index.html'), '<!doctype html><html></html>');
  });

  afterAll(() => {
    if (createdDist) fs.rmSync(distDir, { recursive: true, force: true });
  });

  it('returns 429 once the SPA fallback ceiling is exceeded', async () => {
    process.env.TROGON_ATLAS_STUDIO_STATIC_RATE_MAX = '1';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=spa-rate-limit');
    delete process.env.TROGON_ATLAS_STUDIO_STATIC_RATE_MAX;
    const spaApp = mod.app;

    const first = await request(spaApp).get('/some-page');
    expect(first.status).not.toBe(429);

    const second = await request(spaApp).get('/some-page');
    expect(second.status).toBe(429);
    expect(second.body).toHaveProperty('error');
  });

  it('does not count against the /api rate limiter', async () => {
    process.env.TROGON_ATLAS_STUDIO_STATIC_RATE_MAX = '1';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=spa-rate-limit-api-isolation');
    delete process.env.TROGON_ATLAS_STUDIO_STATIC_RATE_MAX;
    const spaApp = mod.app;

    await request(spaApp).get('/some-page');
    await request(spaApp).get('/some-page');

    const res = await request(spaApp).get('/api/info');
    expect(res.status).toBe(200);
  });

  it('is disabled by the same toggle as the /api limiter', async () => {
    process.env.TROGON_ATLAS_STUDIO_RATE_MAX = '0';
    process.env.TROGON_ATLAS_STUDIO_STATIC_RATE_MAX = '1';
    process.env.NODE_ENV = 'test';
    const mod = await import('./index.mjs?t=spa-rate-limit-disabled');
    delete process.env.TROGON_ATLAS_STUDIO_RATE_MAX;
    delete process.env.TROGON_ATLAS_STUDIO_STATIC_RATE_MAX;
    const spaApp = mod.app;

    const first = await request(spaApp).get('/some-page');
    expect(first.status).not.toBe(429);

    const second = await request(spaApp).get('/some-page');
    expect(second.status).not.toBe(429);
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

  it('forwards by-domain query controls with caller authorization and branch', async () => {
    const invoke = vi.spyOn(fakeClient, 'listEntitiesByDomain').mockImplementation((_req, _meta, _opts, cb) => {
      cb(null, { entities: [], nextPageToken: 'next-domain-page' });
    });
    try {
      const res = await request(ptApp).get('/api/by-domain/acme/checkout')
        .set('Authorization', 'Bearer domain-caller')
        .query({ version: '9007199254740993', latestVersionsOnly: 'false', pageSize: '501', pageToken: 'cursor', branch: 'review/domain' });
      expect(res.status).toBe(200);
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(invoke.mock.calls[0][0]).toEqual({
        domainId: { namespace: 'acme', slug: 'checkout', version: '9007199254740993' },
        kinds: [], latestVersionsOnly: false, pageSize: 501, pageToken: 'cursor',
      });
      expect(invoke.mock.calls[0][1].get('authorization')).toEqual(['Bearer domain-caller']);
      expect(invoke.mock.calls[0][1].get('x-trogon-atlas-branch')).toEqual(['review/domain']);
      expect(res.body).toEqual({ entities: [], nextPageToken: 'next-domain-page' });
    } finally {
      invoke.mockRestore();
    }
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

describe('/-/readiness and /-/liveness', () => {
  /** @param {() => Promise<any>} load @param {Record<string, string>} [env] */
  async function freshServer(load, env = {}) {
    Object.assign(process.env, env, { NODE_ENV: 'test' });
    const mod = await load();
    for (const key of Object.keys(env)) delete process.env[key];
    return mod;
  }

  /** @param {(req: unknown, meta: unknown, opts: unknown, cb: Function) => void} impl @param {() => Promise<void>} fn */
  async function withServerInfo(impl, fn) {
    const original = fakeClient.getServerInfo;
    fakeClient.getServerInfo = impl;
    try {
      await fn();
    } finally {
      fakeClient.getServerInfo = original;
    }
  }

  const unavailable = (_req, _meta, _opts, cb) => cb(Object.assign(new Error('connection refused'), { code: 14 }));
  const unauthenticated = (_req, _meta, _opts, cb) => cb(Object.assign(new Error('unauthenticated'), { code: 16 }));

  it('answers both probes without a token while one is configured', async () => {
    const mod = await freshServer(() => import('./index.mjs?t=health-auth'), { TROGON_ATLAS_AUTH_TOKEN: 'probe-secret' });
    await mod.checkUpstream();
    const readiness = await request(mod.app).get('/-/readiness');
    const liveness = await request(mod.app).get('/-/liveness');
    expect(readiness.status).toBe(200);
    expect(readiness.text).toBe('ok\n');
    expect(liveness.status).toBe(200);
    expect(liveness.text).toBe('ok\n');
  });

  it('is not ready before the first upstream check, while live', async () => {
    const mod = await freshServer(() => import('./index.mjs?t=health-unchecked'));
    const readiness = await request(mod.app).get('/-/readiness');
    expect(readiness.status).toBe(503);
    expect(readiness.text).toBe('not ready\n');
    expect((await request(mod.app).get('/-/liveness')).status).toBe(200);
  });

  it('is not ready when the last upstream check failed, and ready again once one passes', async () => {
    const mod = await freshServer(() => import('./index.mjs?t=health-failed'));
    await mod.checkUpstream();
    expect((await request(mod.app).get('/-/readiness')).status).toBe(200);
    await withServerInfo(unavailable, () => mod.checkUpstream());
    expect((await request(mod.app).get('/-/readiness')).status).toBe(503);
    expect((await request(mod.app).get('/-/liveness')).status).toBe(200);
    await mod.checkUpstream();
    expect((await request(mod.app).get('/-/readiness')).status).toBe(200);
  });

  it('does not call upstream when probed', async () => {
    const mod = await freshServer(() => import('./index.mjs?t=health-held'));
    await mod.checkUpstream();
    let calls = 0;
    await withServerInfo(
      (_req, _meta, _opts, cb) => {
        calls += 1;
        cb(Object.assign(new Error('connection refused'), { code: 14 }));
      },
      async () => {
        expect((await request(mod.app).get('/-/readiness')).status).toBe(200);
      },
    );
    expect(calls).toBe(0);
  });

  it('treats an unauthenticated answer as not ready when the bridge holds a token', async () => {
    const mod = await freshServer(() => import('./index.mjs?t=health-badtoken'), { TROGON_ATLAS_AUTH_TOKEN: 'stale-secret' });
    await withServerInfo(unauthenticated, () => mod.checkUpstream());
    expect((await request(mod.app).get('/-/readiness')).status).toBe(503);
  });

  it('treats an unauthenticated answer as reachable for a pass-through bridge with no token of its own', async () => {
    const mod = await freshServer(() => import('./index.mjs?t=health-anonymous'), { TROGON_ATLAS_AUTH_PASSTHROUGH: 'true' });
    await withServerInfo(unauthenticated, () => mod.checkUpstream());
    expect((await request(mod.app).get('/-/readiness')).status).toBe(200);
  });

  it('lists each check with ?verbose', async () => {
    const mod = await freshServer(() => import('./index.mjs?t=health-verbose'));
    await withServerInfo(unavailable, () => mod.checkUpstream());
    const readiness = await request(mod.app).get('/-/readiness?verbose');
    expect(readiness.status).toBe(503);
    expect(readiness.text).toBe('[+]ping ok\n[-]upstream failed\n[+]shutdown ok\nreadiness check failed\n');
    const liveness = await request(mod.app).get('/-/liveness?a=1&verbose=true');
    expect(liveness.status).toBe(200);
    expect(liveness.text).toBe('[+]ping ok\nliveness check passed\n');
    expect((await request(mod.app).get('/-/liveness?verbosely')).text).toBe('ok\n');
  });

  it('goes not ready on shutdown while liveness keeps passing', async () => {
    const mod = await freshServer(() => import('./index.mjs?t=health-shutdown'));
    await mod.checkUpstream();
    expect((await request(mod.app).get('/-/readiness')).status).toBe(200);
    mod.beginShutdown();
    await mod.checkUpstream();
    const readiness = await request(mod.app).get('/-/readiness?verbose');
    expect(readiness.status).toBe(503);
    expect(readiness.text).toBe('[+]ping ok\n[+]upstream ok\n[-]shutdown failed\nreadiness check failed\n');
    expect((await request(mod.app).get('/-/liveness')).status).toBe(200);
  });

  it('keeps probes out of the access log', async () => {
    const mod = await freshServer(() => import('./index.mjs?t=health-quiet'));
    const write = vi.spyOn(process.stdout, 'write').mockImplementation(() => true);
    try {
      await request(mod.app).get('/-/liveness');
      await request(mod.app).get('/-/readiness');
      const probeLines = write.mock.calls.map(([line]) => String(line));
      expect(probeLines.some((line) => line.includes('"http_request"'))).toBe(false);
      await request(mod.app).get('/api/info');
      const logged = write.mock.calls.map(([line]) => String(line));
      expect(logged.some((line) => line.includes('"http_request"'))).toBe(true);
    } finally {
      write.mockRestore();
    }
  });
});

describe('GET DataProduct backend inspection', () => {
  it('forwards exact product, caller authorization and branch and retains checked evidence', async () => {
    let received;
    fakeClient.inspectDataProduct = (req, meta, _opts, cb) => {
      received = { req, branch: meta.get('x-trogon-atlas-branch'), authorization: meta.get('authorization') };
      cb(null, { etag: 'checked-etag', scope: { branch: 'review', revision: 'checked-context' }, incomplete: true, producing: [{ path: [{ id: { value: 'model-edge' } }] }] });
    };
    process.env.TROGON_ATLAS_AUTH_PASSTHROUGH = 'true';
    const module = await import('./index.mjs?t=data-product-passthrough');
    delete process.env.TROGON_ATLAS_AUTH_PASSTHROUGH;
    const res = await request(module.app).get('/api/data-product/acme/offering/inspect?version=3&branch=review').set('Authorization', 'Bearer caller-token');
    expect(res.status).toBe(200);
    expect(received).toEqual({ req: { product: { id: { namespace: 'acme', slug: 'offering', version: '3' } } }, branch: ['review'], authorization: ['Bearer caller-token'] });
    expect(res.body).toMatchObject({ etag: 'checked-etag', incomplete: true, scope: { revision: 'checked-context' }, producing: [{ path: [{ id: { value: 'model-edge' } }] }] });
    delete fakeClient.inspectDataProduct;
  });
  it.each(['', '0', '-1', 'latest', '1.5'])('refuses a nonexact version %s before upstream inspection', async (version) => {
    const inspect = vi.fn();
    fakeClient.inspectDataProduct = inspect;
    const res = await request(app).get(`/api/data-product/acme/offering/inspect?version=${version}`);
    expect(res.status).toBe(400);
    expect(inspect).not.toHaveBeenCalled();
    delete fakeClient.inspectDataProduct;
  });
  it('preserves backend operational errors instead of converting them to undeclared data', async () => {
    fakeClient.inspectDataProduct = (_req, _meta, _opts, cb) => cb(Object.assign(new Error('backend unavailable'), { code: 14 }));
    const res = await request(app).get('/api/data-product/acme/offering/inspect?version=1');
    expect(res.status).toBe(503);
    expect(res.body.error).toContain('backend unavailable');
    delete fakeClient.inspectDataProduct;
  });
});

describe('GET responsibility queries', () => {
  const id = { namespace: 'acme', slug: 'catalog', version: '7' };
  const member = { member: { specification: { id }, ownerUid: 'exact-owner', kind: 'CONTRACT_MEMBER_KIND_ARTIFACT', memberId: { value: 'member-id' }, expectedDigest: { sha256: 'a'.repeat(64) } } };
  it.each([
    ['subject-assignments', 'listSubjectAssignments', { subject: member, kind: 'RESPONSIBILITY_KIND_STEWARD' }],
    ['subject-assignments', 'listSubjectAssignments', { subject: { connection: { owner: { kind: 'ENTITY_KIND_AUTOMATION_SLICE', id }, ownerUid: 'occurrence-owner', id: { value: 'edge-b' } } } }],
    ['subject-assignments', 'listSubjectAssignments', { subject: { port: { product: { id }, ownerUid: 'product-owner', direction: 'DATA_PORT_DIRECTION_INPUT', portId: { value: 'input-b' } } } }],
    ['assignee-responsibilities', 'listAssigneeResponsibilities', { assignee: { person: { id } } }],
    ['team-members', 'listTeamMembers', { team: { id } }],
    ['person-teams', 'listPersonTeams', { person: { id } }],
    ['team-contacts', 'getTeamContacts', { team: { id }, includeMembers: true }],
  ])('forwards %s with exact typed request, authorization and checked context', async (route, method, input) => {
    const invoke = vi.fn((req, meta, _opts, cb) => cb(null, { scope: { branch: 'review', revision: 'checked', visitedSubjects: [member] }, incomplete: true, subjectState: 'RESPONSIBILITY_RESOLUTION_STATE_INACCESSIBLE' }));
    fakeClient[method] = invoke;
    process.env.TROGON_ATLAS_AUTH_PASSTHROUGH = 'true';
    const module = await import('./index.mjs?t=responsibility-passthrough');
    delete process.env.TROGON_ATLAS_AUTH_PASSTHROUGH;
    try {
      const response = await request(module.app).get(`/api/responsibility/${route}`).query({ request: JSON.stringify(input), branch: 'review' }).set('Authorization', 'Bearer caller-token');
      expect(response.status).toBe(200);
      expect(invoke.mock.calls[0][0]).toEqual(input);
      expect(invoke.mock.calls[0][1].get('authorization')).toEqual(['Bearer caller-token']);
      expect(invoke.mock.calls[0][1].get('x-trogon-atlas-branch')).toEqual(['review']);
      expect(response.body).toMatchObject({ incomplete: true, scope: { revision: 'checked', visitedSubjects: [member] }, subjectState: 'RESPONSIBILITY_RESOLUTION_STATE_INACCESSIBLE' });
    } finally { delete fakeClient[method]; }
  });
  it.each(['', '[]', 'null', 'broken'])('rejects malformed request %s', async (input) => {
    const response = await request(app).get('/api/responsibility/subject-assignments').query({ request: input });
    expect(response.status).toBe(400);
  });
  it('refuses unsupported queries, historical hydration and writes', async () => {
    expect((await request(app).get('/api/responsibility/putEntity').query({ request: '{}' })).status).toBe(400);
    expect((await request(app).get('/api/responsibility/team-members').query({ request: '{}', asOf: 'historical' })).status).toBe(400);
    expect((await request(app).post('/api/responsibility/team-members').send({ team: { id } })).status).toBe(403);
  });
  it.each([[14, 503], [8, 429], [7, 403], [5, 404]])('preserves public backend error class %s', async (code, expected) => {
    fakeClient.listSubjectAssignments = (_req, _meta, _opts, cb) => cb(Object.assign(new Error('query failed'), { code }));
    try {
      const response = await request(app).get('/api/responsibility/subject-assignments').query({ request: JSON.stringify({ subject: member }) });
      expect(response.status).toBe(expected);
      expect(response.body.error).toContain('query failed');
      expect(response.body.assignments).toBeUndefined();
    } finally { delete fakeClient.listSubjectAssignments; }
  });
});


describe('responsibility strict production serializer boundary', () => {
  const id = { namespace: 'acme', slug: 'catalog', version: '9007199254740993' };
  const subject = { entity: { kind: 'ENTITY_KIND_EVENT', id } };
  const team = { id };
  const invalid = [
    ['unknown filter name', 'subject-assignments', 'ListSubjectAssignments', { subject, kind: 'TYPO' }],
    ['unknown numeric filter', 'subject-assignments', 'ListSubjectAssignments', { subject, kind: 999 }],
    ['boolean filter', 'subject-assignments', 'ListSubjectAssignments', { subject, kind: true }],
    ['string false expansion', 'team-contacts', 'GetTeamContacts', { team, includeMembers: 'false' }],
    ['numeric expansion', 'team-contacts', 'GetTeamContacts', { team, includeMembers: 0 }],
    ['null expansion', 'team-contacts', 'GetTeamContacts', { team, includeMembers: null }],
    ['unknown request field', 'team-contacts', 'GetTeamContacts', { team, includeMembers: false, unknown: true }],
    ['unknown nested field', 'team-members', 'ListTeamMembers', { team: { id, extra: 'ignored' } }],
    ['multiple subject alternatives', 'subject-assignments', 'ListSubjectAssignments', { subject: { ...subject, port: {} } }],
    ['multiple assignee alternatives', 'assignee-responsibilities', 'ListAssigneeResponsibilities', { assignee: { team, person: team } }],
    ['null selector alternative', 'subject-assignments', 'ListSubjectAssignments', { subject: { entity: null } }],
    ['unknown entity kind', 'subject-assignments', 'ListSubjectAssignments', { subject: { entity: { kind: 'TYPO', id } } }],
    ['unknown direction', 'subject-assignments', 'ListSubjectAssignments', { subject: { port: { product: team, direction: 'TYPO' } } }],
    ['unknown member kind', 'subject-assignments', 'ListSubjectAssignments', { subject: { member: { specification: team, kind: 99 } } }],
    ['numeric owner UID', 'subject-assignments', 'ListSubjectAssignments', { subject: { connection: { owner: subject.entity, ownerUid: 1, id: { value: 'edge' } } } }],
    ['snake case pin silently lost', 'subject-assignments', 'ListSubjectAssignments', { subject: { member: { specification: team, kind: 'CONTRACT_MEMBER_KIND_ARTIFACT', expected_digest: { sha256: 'a'.repeat(64) } } } }],
    ['unsafe numeric version', 'team-members', 'ListTeamMembers', { team: { id: { ...id, version: 9007199254740992 } } }],
    ['fractional version', 'team-members', 'ListTeamMembers', { team: { id: { ...id, version: 1.5 } } }],
    ['overflow version', 'team-members', 'ListTeamMembers', { team: { id: { ...id, version: '18446744073709551616' } } }],
    ['negative version', 'team-members', 'ListTeamMembers', { team: { id: { ...id, version: -1 } } }],
    ['boolean version', 'team-members', 'ListTeamMembers', { team: { id: { ...id, version: true } } }],
  ];
  it.each(invalid)('rejects %s before permissive production serialization', async (_label, route, rpc, input) => {
    const loader = (await vi.importActual('@grpc/proto-loader')).default;
    const protoRoot = path.resolve(here, '../../../../proto');
    const definitions = loader.loadSync(path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'), { keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [protoRoot] });
    const method = Object.values(definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService']).find((value) => value.path.endsWith(`/${rpc}`));
    const codec = definitions[`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`];
    const serialize = vi.fn((req, _meta, _opts, cb) => {
      try { cb(null, { received: codec.deserialize(method.requestSerialize(req)) }); }
      catch (error) { cb(error); }
    });
    const name = rpc[0].toLowerCase() + rpc.slice(1);
    fakeClient[name] = serialize;
    try {
      const response = await request(app).get(`/api/responsibility/${route}`).query({ request: JSON.stringify(input) });
      expect(response.status).toBe(400);
      expect(serialize).not.toHaveBeenCalled();
    } finally { delete fakeClient[name]; }
  });
  it.each([
    ...['ENTITY_KIND_COMPONENT', 'ENTITY_KIND_EXTERNAL_SYSTEM'].map((kind) => {
      const input = { subject: { connection: { owner: { kind, id }, ownerUid: 'opaque-owner-UID', id: { value: 'occurrence-b' } } } };
      return ['subject-assignments', 'ListSubjectAssignments', input, input];
    }),
    ['subject-assignments', 'ListSubjectAssignments', { subject, kind: 'RESPONSIBILITY_KIND_CONTACT' }, { subject, kind: 'RESPONSIBILITY_KIND_CONTACT' }],
    ['subject-assignments', 'ListSubjectAssignments', { subject: { entity: { kind: 1, id } }, kind: 4 }, { subject, kind: 'RESPONSIBILITY_KIND_CONTACT' }],
    ['subject-assignments', 'ListSubjectAssignments', { subject, kind: 0 }, { subject, kind: 'RESPONSIBILITY_KIND_UNSPECIFIED' }],
    ['team-contacts', 'GetTeamContacts', { team, includeMembers: false }, { team, includeMembers: false }],
    ['team-contacts', 'GetTeamContacts', { team }, { team }],
  ])('preserves valid %s through the actual gateway and production serializer', async (route, rpc, input, expected) => {
    const loader = (await vi.importActual('@grpc/proto-loader')).default;
    const protoRoot = path.resolve(here, '../../../../proto');
    const definitions = loader.loadSync(path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'), { keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [protoRoot] });
    const method = Object.values(definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService']).find((value) => value.path.endsWith(`/${rpc}`));
    const codec = definitions[`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`];
    const invoke = vi.fn((req, _meta, _opts, cb) => cb(null, { received: codec.deserialize(method.requestSerialize(req)) }));
    const name = rpc[0].toLowerCase() + rpc.slice(1);
    fakeClient[name] = invoke;
    try {
      const response = await request(app).get(`/api/responsibility/${route}`).query({ request: JSON.stringify(input) });
      expect(response.status).toBe(200);
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(invoke.mock.calls[0][0]).toEqual(input);
      expect(response.body.received).toEqual(expected);
    } finally { delete fakeClient[name]; }
  });

});


describe('catalog strict gateway serializer boundary', () => {
  const id = { namespace: 'catalog', slug: 'note', version: '9007199254740993' };
  const document = { anchor: { document: { id }, ownerUid: 'incarnation', anchorId: { value: 'anchor-b' }, expectedDigest: { sha256: 'a'.repeat(64) } } };
  const subject = { entity: { kind: 'ENTITY_KIND_EVENT', id } };
  it.each([
    ...[null, '1', -1, 1.5, 4294967296].map((pageSize) => ['decision-amendments', 'ListDecisionAmendments', { decision: { id }, direction: 'DECISION_AMENDMENT_DIRECTION_INCOMING', pageSize }]),
    ['subject-decisions', 'ListSubjectDecisions', { subject, status: 'TYPO' }],
    ['subject-decisions', 'ListSubjectDecisions', { subject, status: 991 }],
    ['decision-detail', 'GetDecisionDetail', { decision: { id }, followReplacement: 'false' }],
    ['decision-detail', 'GetDecisionDetail', { decision: { id }, followReplacement: null }],
    ['document-detail', 'GetDocumentDetail', { document: {} }],
    ['document-detail', 'GetDocumentDetail', { document: { ...document, document: { id } } }],
    ['document-detail', 'GetDocumentDetail', { document: { anchor: { ...document.anchor, owner_uid: 'lost' } } }],
    ['incoming-documents', 'ListIncomingDocumentDeclarations', { document, unknown: true }],
  ])('rejects malformed %s before calling serializer', async (route, rpc, input) => {
    const name = rpc[0].toLowerCase() + rpc.slice(1);
    const invoke = vi.fn();
    fakeClient[name] = invoke;
    try {
      const response = await request(app).get(`/api/catalog/${route}`).query({ request: JSON.stringify(input) });
      expect(response.status).toBe(400);
      expect(invoke).not.toHaveBeenCalled();
    } finally { delete fakeClient[name]; }
  });
  it.each([
    ...['ENTITY_KIND_COMPONENT', 'ENTITY_KIND_EXTERNAL_SYSTEM'].flatMap((kind) => ['subject-decisions', 'subject-documents'].map((route) => [route, route === 'subject-decisions' ? 'ListSubjectDecisions' : 'ListSubjectDocuments', { subject: { connection: { owner: { kind, id }, ownerUid: 'opaque-owner-UID', id: { value: 'occurrence-b' } } } }])),
    ['decision-amendments', 'ListDecisionAmendments', { decision: { id }, direction: 'DECISION_AMENDMENT_DIRECTION_INCOMING', pageSize: 1, pageToken: 'opaque-token' }],
    ['subject-decisions', 'ListSubjectDecisions', { subject, status: 'DECISION_STATUS_ACCEPTED' }],
    ['subject-documents', 'ListSubjectDocuments', { subject }],
    ['decision-detail', 'GetDecisionDetail', { decision: { id }, followReplacement: false }],
    ['document-detail', 'GetDocumentDetail', { document }],
    ['incoming-decisions', 'ListIncomingDecisionDeclarations', { decision: { id } }],
    ['incoming-documents', 'ListIncomingDocumentDeclarations', { document }],
  ])('preserves valid %s through HTTP and real serializer', async (route, rpc, input) => {
    const loader = (await vi.importActual('@grpc/proto-loader')).default;
    const protoRoot = path.resolve(here, '../../../../proto');
    const definitions = loader.loadSync(path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'), { keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [protoRoot] });
    const method = Object.values(definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService']).find(value => value.path.endsWith(`/${rpc}`));
    const codec = definitions[`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`];
    const invoke = vi.fn((req, meta, _opts, cb) => cb(null, { received: codec.deserialize(method.requestSerialize(req)), branch: meta.get('x-trogon-atlas-branch')[0], incomplete: true, failures: [{ code: 14, message: 'Unavailable' }] }));
    const name = rpc[0].toLowerCase() + rpc.slice(1);
    fakeClient[name] = invoke;
    try {
      const response = await request(app).get(`/api/catalog/${route}`).query({ request: JSON.stringify(input), branch: 'review' });
      expect(response.status).toBe(200);
      expect(response.body.received).toEqual(input);
      expect(response.body.branch).toBe('review');
      expect(response.body.incomplete).toBe(true);
      expect(response.body.failures[0].code).toBe(14);
    } finally { delete fakeClient[name]; }
  });
  it('refuses unknown routes, mutations and historical hydration', async () => {
    expect((await request(app).get('/api/catalog/putEntity').query({ request: '{}' })).status).toBe(400);
    expect((await request(app).get('/api/catalog/document-detail').query({ request: '{}', asOf: 'historical' })).status).toBe(400);
    expect((await request(app).post('/api/catalog/document-detail').send({ document })).status).toBe(403);
  });
});


describe('source read-only gateway', () => {
  it.each([
    ['snapshot', 'GetSourceSnapshot', { registrationId: 'r', snapshotId: 's', includeBytes: false }],
    ['overlay', 'GetSourceOverlay', { registrationId: 'r', snapshotId: 's', pageSize: 100 }],
    ['run-results', 'ListSourceRunResults', { registrationId: 'r', runId: 'run', pageSize: 100, pageToken: 'opaque' }],
    ['run-operations', 'ListSourceRunOperations', { registrationId: 'r', runId: 'run', pageSize: 100 }],
  ])('forwards %s through the actual protobuf serializer and caller branch', async (route, rpc, input) => {
    const loader = (await vi.importActual('@grpc/proto-loader')).default;
    const root = path.resolve(here, '../../../../proto');
    const definitions = loader.loadSync(path.join(root, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'), { keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [root] });
    const method = Object.values(definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService']).find(value => value.path.endsWith(`/${rpc}`));
    const codec = definitions[`trogonatlas.api.eventmodel.v1alpha1.${rpc}Request`];
    const name = rpc[0].toLowerCase() + rpc.slice(1);
    fakeClient[name] = vi.fn((req, meta, _opts, cb) => cb(null, { received: codec.deserialize(method.requestSerialize(req)), branch: meta.get('x-trogon-atlas-branch')[0], incomplete: true, nextPageToken: 'more' }));
    try {
      const response = await request(app).get(`/api/source/${route}`).query({ request: JSON.stringify(input), branch: 'source-review' });
      expect(response.status).toBe(200);
      expect(response.body.received).toEqual(input);
      expect(response.body.branch).toBe('source-review');
      expect(response.body.incomplete).toBe(true);
      expect(response.body.nextPageToken).toBe('more');
    } finally { delete fakeClient[name]; }
  });
  it.each([
    ['snapshot', { registrationId: 'r', includeBytes: 'false' }],
    ['snapshot', { registrationId: 'r', surprise: true }],
    ['runs', { registrationId: 'r', pageSize: '100' }],
  ])('refuses malformed %s before calling the backend', async (route, input) => {
    const name = route === 'runs' ? 'listSourceRuns' : 'getSourceSnapshot';
    const invoke = vi.fn(); fakeClient[name] = invoke;
    try {
      const response = await request(app).get(`/api/source/${route}`).query({ request: JSON.stringify(input) });
      expect(response.status).toBe(400); expect(invoke).not.toHaveBeenCalled();
    } finally { delete fakeClient[name]; }
  });
  it('exposes no source mutation route or historical hydration', async () => {
    for (const query of ['apply', 'recover', 'cancel', 'preview', 'create-registration']) expect((await request(app).get(`/api/source/${query}`).query({ request: '{}' })).status).toBe(400);
    expect((await request(app).get('/api/source/run').query({ request: '{"registrationId":"r","runId":"x"}', asOf: 'old' })).status).toBe(400);
  });
});

describe('design topology gateway response codec', () => {
  const version = '9007199254740993';
  const graphId = { namespace: 'topology', slug: 'flow', version };
  const step = (value) => ({ value });
  const owner = { owner: { kind: 'ENTITY_KIND_DESIGN_GRAPH', id: graphId }, ownerUid: 'incarnation-1' };
  const scope = { revision: '42', authorizedNamespaces: ['topology'], visited: [owner] };
  const loadCodec = async (rpc) => {
    const loader = (await vi.importActual('@grpc/proto-loader')).default;
    const protoRoot = path.resolve(here, '../../../../proto');
    const definitions = loader.loadSync(path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'), { keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [protoRoot] });
    const method = Object.values(definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService']).find((value) => value.path.endsWith(`/${rpc}`));
    return (message) => method.responseDeserialize(method.responseSerialize(message));
  };
  it.each([
    ['design-graph-detail', 'GetDesignGraphDetail', { graph: { id: graphId } }, {
      entity: { designGraph: { title: { value: 'Flow' }, steps: [{ stepId: step('step-a') }, { stepId: step('step-b') }] } },
      etag: 'etag-1',
      targets: [
        { stepId: step('step-a'), field: 'steps[0].target', state: 'CATALOG_RESOLUTION_STATE_RESOLVED', owner },
        { stepId: step('step-b'), field: 'steps[1].target', state: 'CATALOG_RESOLUTION_STATE_UNAVAILABLE' },
      ],
      layoutState: 'CATALOG_RESOLUTION_STATE_UNAVAILABLE',
      scope,
      incomplete: true,
      failures: [{ field: 'layout', code: 14, message: 'Unavailable' }],
    }],
    ['subject-design-graphs', 'ListSubjectDesignGraphs', { subject: { entity: { kind: 'ENTITY_KIND_COMPONENT', id: graphId } } }, {
      scope,
      subjectState: 'CATALOG_RESOLUTION_STATE_RESOLVED',
    }],
    ['design-topology', 'TraverseDesignGraphTopology', { root: { id: graphId }, followSubflows: true, maxDepth: 4 }, {
      nodes: [{ graph: owner, stepId: step('step-a'), depth: 0, authoredEntry: true, targetState: 'CATALOG_RESOLUTION_STATE_RESOLVED' }, { graph: owner, stepId: step('step-b'), depth: 1 }],
      edges: [
        { kind: 'KIND_TRANSITION', graph: owner, fromStepId: step('step-a'), connectionId: step('edge-1'), label: { value: 'next' }, toStepId: step('step-b') },
        { kind: 'KIND_TRANSITION', graph: owner, fromStepId: step('step-a'), connectionId: step('edge-2'), label: { value: 'next' }, toStepId: step('step-b') },
        { kind: 'KIND_SUBFLOW', graph: owner, fromStepId: step('step-b'), toGraph: { id: { ...graphId, slug: 'callee' } } },
      ],
      subflows: [{ graph: owner, stepId: step('step-b'), subflow: { id: { ...graphId, slug: 'callee' } }, state: 'TOPOLOGY_SUBFLOW_STATE_UNAVAILABLE', depth: 1 }],
      scope,
      appliedMaxDepth: 4,
      followSubflows: true,
    }],
    ['incoming-subflows', 'ListIncomingDesignGraphSubflows', { graph: { id: graphId } }, {
      references: [{ owner, field: 'steps[1].target.subflow', state: 'CATALOG_RESOLUTION_STATE_RESOLVED', stepId: step('step-b'), subflow: { id: graphId } }],
      scope,
      graphState: 'CATALOG_RESOLUTION_STATE_RESOLVED',
    }],
  ])('returns the production %s response codec output unchanged with identities, presence and unavailable states', async (route, rpc, input, response) => {
    const decode = await loadCodec(rpc);
    const decoded = decode(response);
    const name = rpc[0].toLowerCase() + rpc.slice(1);
    fakeClient[name] = vi.fn((_req, _meta, _opts, cb) => cb(null, decoded));
    try {
      const result = await request(app).get(`/api/catalog/${route}`).query({ request: JSON.stringify(input), branch: 'review' });
      expect(result.status).toBe(200);
      expect(result.body).toEqual(JSON.parse(JSON.stringify(decoded)));
      expect(JSON.stringify(result.body)).toContain(version);
      expect(JSON.stringify(result.body)).not.toMatch(/"version":9007/);
      expect(result.body.scope.revision).toBe('42');
      if (route === 'design-graph-detail') {
        expect(result.body.layoutState).toBe('CATALOG_RESOLUTION_STATE_UNAVAILABLE');
        expect(result.body.targets[1].state).toBe('CATALOG_RESOLUTION_STATE_UNAVAILABLE');
        expect(result.body.targets[1]).not.toHaveProperty('owner');
        expect(result.body.incomplete).toBe(true);
        expect(result.body.failures[0].code).toBe(14);
      }
      if (route === 'design-topology') {
        expect(result.body.edges.map((edge) => edge.connectionId?.value)).toEqual(['edge-1', 'edge-2', undefined]);
        expect(result.body.nodes[1]).not.toHaveProperty('authoredEntry');
        expect(result.body.subflows[0].state).toBe('TOPOLOGY_SUBFLOW_STATE_UNAVAILABLE');
        expect(result.body).not.toHaveProperty('incomplete');
      }
      if (route === 'incoming-subflows') expect(result.body.references[0].stepId.value).toBe('step-b');
    } finally { delete fakeClient[name]; }
  });
  it('omits absent layout state, incompleteness and failures instead of inventing defaults', async () => {
    const decode = await loadCodec('GetDesignGraphDetail');
    const decoded = decode({ entity: { designGraph: { title: { value: 'Flow' } } }, etag: 'e', scope });
    fakeClient.getDesignGraphDetail = vi.fn((_req, _meta, _opts, cb) => cb(null, decoded));
    try {
      const result = await request(app).get('/api/catalog/design-graph-detail').query({ request: JSON.stringify({ graph: { id: graphId } }) });
      expect(result.status).toBe(200);
      for (const field of ['layoutState', 'incomplete', 'failures', 'targets', 'declarations']) expect(result.body).not.toHaveProperty(field);
    } finally { delete fakeClient.getDesignGraphDetail; }
  });
});

describe('native human interaction gateway', () => {
  const version = '9007199254740993';
  const id = (slug) => ({ namespace: 'review', slug, version });
  const selected = (kind, slug, value) => ({ owner: { kind, id: id(slug) }, ownerUid: `${slug}-uid`, id: { value } });
  const hosting = selected('ENTITY_KIND_COMPONENT', 'portal', 'hosting-a');
  const input = {
    hosting,
    display: {
      persona: selected('ENTITY_KIND_UI_SLICE', 'display', 'persona-a'),
      ui: selected('ENTITY_KIND_UI_SLICE', 'display', 'ui-a'),
      sourceReadModel: selected('ENTITY_KIND_UI_SLICE', 'display', 'observation-b'),
    },
  };
  it('forwards only the supplied tuple with auth/branch and preserves checked response through real codecs', async () => {
    const loader = (await vi.importActual('@grpc/proto-loader')).default;
    const root = path.resolve(here, '../../../../proto');
    const definitions = loader.loadSync(path.join(root, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'), { keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [root] });
    const method = Object.values(definitions['trogonatlas.api.eventmodel.v1alpha1.EventModelService']).find((value) => value.path.endsWith('/InspectHumanInteraction'));
    const reqCodec = definitions['trogonatlas.api.eventmodel.v1alpha1.InspectHumanInteractionRequest'];
    const output = {
      request: input, state: 'CATALOG_RESOLUTION_STATE_UNAVAILABLE',
      consistency: 'HUMAN_INTERACTION_CONSISTENCY_PER_OWNER_CHECKED_READ_SET',
      scope: { branch: 'reviewer/check', revision: 'read-set-digest', authorizedNamespaces: ['review'], visited: [{ owner: hosting.owner, uid: hosting.ownerUid, etag: 'component-revision' }] },
      edges: [{ role: 'HUMAN_INTERACTION_ROLE_READ_MODEL', selection: input.display.sourceReadModel, field: 'source_read_models[1]', target: { kind: 'ENTITY_KIND_READ_MODEL', id: id('summary') }, state: 'CATALOG_RESOLUTION_STATE_UNAVAILABLE' }],
    };
    fakeClient.inspectHumanInteraction = vi.fn((body, _meta, _opts, cb) => {
      expect(reqCodec.deserialize(method.requestSerialize(body))).toEqual(input);
      cb(null, method.responseDeserialize(method.responseSerialize(output)));
    });
    process.env.TROGON_ATLAS_AUTH_PASSTHROUGH = 'true';
    const module = await import('./index.mjs?t=human-interaction-passthrough');
    delete process.env.TROGON_ATLAS_AUTH_PASSTHROUGH;
    try {
      const response = await request(module.app).get('/api/catalog/human-interaction').query({ request: JSON.stringify(input), branch: 'reviewer/check' }).set('Authorization', 'Bearer caller-token');
      expect(response.status).toBe(200);
      expect(response.body).toEqual(output);
      expect(fakeClient.inspectHumanInteraction.mock.calls[0][1].get('authorization')).toEqual(['Bearer caller-token']);
      expect(fakeClient.inspectHumanInteraction.mock.calls[0][1].get('x-trogon-atlas-branch')).toEqual(['reviewer/check']);
      expect(response.body.edges[0]).not.toHaveProperty('checkedTarget');
      expect(response.body).not.toHaveProperty('snapshotId');
    } finally { delete fakeClient.inspectHumanInteraction; }
  });
  it.each([
    ['multiple paths', { ...input, issuance: { persona: input.display.persona, ui: input.display.ui, command: input.display.sourceReadModel } }],
    ['numeric lossy version', { ...input, hosting: { ...hosting, owner: { ...hosting.owner, id: { ...id('portal'), version: 9007199254740993 } } } }],
    ['unknown nested property', { ...input, hosting: { ...hosting, title: 'discarded' } }],
  ])('refuses %s without calling the backend', async (_name, body) => {
    fakeClient.inspectHumanInteraction = vi.fn();
    try {
      const response = await request(app).get('/api/catalog/human-interaction').query({ request: JSON.stringify(body) });
      expect(response.status).toBe(400);
      expect(fakeClient.inspectHumanInteraction).not.toHaveBeenCalled();
    } finally { delete fakeClient.inspectHumanInteraction; }
  });
  it('does not allow historical hydration or mutation through this inspection route', async () => {
    expect((await request(app).get('/api/catalog/human-interaction').query({ request: JSON.stringify(input), asOf: 'old' })).status).toBe(400);
    expect((await request(app).post('/api/catalog/human-interaction').send(input)).status).toBe(403);
  });
});

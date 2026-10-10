// A minimal, hermetic stand-in for the trogon-atlas gRPC server. Loaded
// through the same `service.proto` bundle the real bridge (server/index.mjs)
// uses, so it is wire-compatible with the client the bridge constructs --
// but it never touches Docker, NATS, or the Rust server. This lets e2e
// specs boot the *real* bridge process (real Express app, real proto
// decoding, real HTTP, real shared/safe-namespace.mjs import) fronting a
// backend that returns deterministic fixture data instead of live/shared
// data.
//
// Only the RPCs the bridge actually calls (see server/index.mjs) are
// implemented. Anything else the studio might call falls through to
// `UNIMPLEMENTED`, which surfaces immediately as a loud gRPC error rather
// than a silent success -- if a spec needs a new RPC, add it here.

import path from 'node:path';
import { fileURLToPath } from 'node:url';
import grpc from '@grpc/grpc-js';
import protoLoader from '@grpc/proto-loader';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PROTO_PATH = path.resolve(__dirname, '../../../../../proto/trogonatlas/api/eventmodel/v1alpha1/service.proto');
const PROTO_INCLUDES = [path.resolve(__dirname, '../../../../../proto')];

export const ORDER_MODEL_ID = { namespace: 'shop', slug: 'order-service', version: '1' };

export const ORDER_PLACED_EVENT = {
  event: {
    id: { namespace: 'shop', slug: 'order.placed', version: '1' },
    title: 'Order Placed',
    doc: 'Emitted once a shopper completes checkout.',
    swimlane: null,
    metadata: [],
  },
};

export const ORDER_EVENT_MODEL = {
  eventModel: {
    id: ORDER_MODEL_ID,
    title: 'Order Service',
    doc: 'Core order lifecycle model.',
    members: [{ kind: 'ENTITY_KIND_EVENT', id: ORDER_PLACED_EVENT.event.id }],
    metadata: [],
  },
};

// A single fixture branch, standing in for one that would be created
// through the CLI/gRPC CreateBranch RPC before the suite runs -- the studio
// bridge itself never exposes a create-branch HTTP route (see the comment
// on ListBranches in server/index.mjs: CreateBranch/ListBranches/DeleteBranch
// are exempt from branch-context resolution and the bridge only proxies the
// read side). Its diff has one entry so the conflict drawer has something
// to render.
export const FIXTURE_BRANCH = {
  name: 'e2e/smoke-fixture',
  doc: 'Throwaway branch for the Playwright smoke suite.',
  createdAt: '2026-01-01T00:00:00Z',
  deltaCount: 1,
};

const DIFF_ENTRY = {
  ref: { kind: 'ENTITY_KIND_EVENT', id: ORDER_PLACED_EVENT.event.id },
  status: 'STATUS_CHANGED',
  base: ORDER_PLACED_EVENT,
  baseEtag: 'etag-1',
  ours: { event: { ...ORDER_PLACED_EVENT.event, doc: 'Edited on the branch.' } },
  theirs: ORDER_PLACED_EVENT,
  conflictFieldPaths: ['doc'],
};

function listEntities(call, callback) {
  const kinds = call.request.kinds ?? [];
  const namespaces = call.request.namespaces ?? [];
  const wantsEventModel = kinds.length === 0 || kinds.includes('ENTITY_KIND_EVENT_MODEL') || kinds.includes(12);
  if (!wantsEventModel) {
    callback(null, { entities: [], nextPageToken: '' });
    return;
  }
  // Honor ListEntitiesRequest.namespaces the same way the real server
  // does: empty = all; otherwise the fixture must match one of them.
  if (
    namespaces.length > 0 &&
    !namespaces.includes(ORDER_MODEL_ID.namespace)
  ) {
    callback(null, { entities: [], nextPageToken: '' });
    return;
  }
  callback(null, {
    entities: [ORDER_EVENT_MODEL],
    nextPageToken: '',
  });
}

function getEntity(call, callback) {
  const { kind, id } = call.request;
  const isEventModel = kind === 'ENTITY_KIND_EVENT_MODEL' || kind === 12;
  const isEvent = kind === 'ENTITY_KIND_EVENT' || kind === 1;
  if (isEventModel && id?.namespace === ORDER_MODEL_ID.namespace && id?.slug === ORDER_MODEL_ID.slug) {
    callback(null, { entity: ORDER_EVENT_MODEL });
    return;
  }
  if (isEvent && id?.slug === ORDER_PLACED_EVENT.event.id.slug) {
    callback(null, { entity: ORDER_PLACED_EVENT });
    return;
  }
  callback(null, { entity: null });
}

function listEntitiesByDomain(_call, callback) {
  callback(null, { entities: [] });
}

function listBranches(_call, callback) {
  callback(null, { branches: [FIXTURE_BRANCH] });
}

function diffBranch(call, callback) {
  if (call.request.name !== FIXTURE_BRANCH.name) {
    callback(null, { entries: [] });
    return;
  }
  callback(null, { entries: [DIFF_ENTRY] });
}

function listValidationRules(_call, callback) {
  callback(null, { rules: [] });
}

function listEntityKinds(_call, callback) {
  callback(null, { kinds: [] });
}

function searchEntities(_call, callback) {
  callback(null, { results: [] });
}

function validateEventModel(call, callback) {
  // Dedicated slug so contract tests can observe the `issues` field name
  // under defaults:false (empty repeated fields are omitted on the wire).
  const slug = call.request.eventModelId?.slug;
  if (slug === 'with-validation-issue') {
    callback(null, {
      issues: [{ code: 'E2E_SENTINEL', message: 'fixture sentinel', severity: 'SEVERITY_INFO' }],
    });
    return;
  }
  if (slug === ORDER_MODEL_ID.slug) {
    callback(null, {
      issues: [
        { severity: 'SEVERITY_ERROR', code: 'E2E_MISSING_SOURCE', message: 'Customer identifier needs a source.' },
        { severity: 'SEVERITY_WARNING', code: 'E2E_MISSING_DESCRIPTION', message: 'Explain the customer identifier.' },
        { severity: 'SEVERITY_INFO', code: 'E2E_SCHEMA_HINT', message: 'Declare the payload schema.' },
      ].map((issue) => ({
        ...issue,
        subject: { kind: 'ENTITY_KIND_EVENT', id: ORDER_PLACED_EVENT.event.id },
        subjectField: 'schema.fields.customer_id',
      })),
    });
    return;
  }
  callback(null, { issues: [] });
}

function validateProject(_call, callback) {
  callback(null, { reports: [], totalErrors: 0, totalWarnings: 0, totalInfo: 0 });
}

function listChanges(_call, callback) {
  callback(null, { events: [], nextToken: '' });
}

function deleteByQuery(call, callback) {
  // Proto: repeated Deleted deleted + int32 deleted_count. A bare number
  // in `deleted` is the wrong type and is stripped on the wire.
  const namespace = call.request.namespace;
  const slug = call.request.slug;
  if (namespace === ORDER_PLACED_EVENT.event.id.namespace && slug === ORDER_PLACED_EVENT.event.id.slug) {
    callback(null, {
      deleted: [
        {
          kind: 'ENTITY_KIND_EVENT',
          id: ORDER_PLACED_EVENT.event.id,
        },
      ],
      deletedCount: 1,
    });
    return;
  }
  callback(null, { deleted: [], deletedCount: 0 });
}

/** Reports a diagnostic for any file without a syntax line, so e2e can show both outcomes. */
function compileTypeLibrary(call, callback) {
  const files = call.request.library?.files ?? [];
  const diagnostics = files
    .filter((f) => !String(f.content ?? '').includes('syntax'))
    .map((f) => ({ path: f.path, line: 1, column: 1, message: 'expected syntax declaration' }));
  callback(null, { diagnostics, compatibilityViolations: [] });
}

function getTypeLibraryDescriptorSet(call, callback) {
  const messages =
    call.request.namespace === ORDER_MODEL_ID.namespace
      ? [{ fullName: 'shop.orders.v1.OrderPlaced', library: { namespace: 'shop', slug: 'shop.orders.v1', version: '1' } }]
      : [];
  callback(null, { fileDescriptorSet: Buffer.alloc(0), messages });
}

function unimplemented(_call, callback) {
  callback({ code: grpc.status.UNIMPLEMENTED, message: 'not implemented in the e2e fixture server' });
}

/**
 * Starts the fixture gRPC server on an OS-assigned loopback port.
 * @returns {Promise<{ url: string; stop: () => Promise<void> }>}
 */
export async function startFakeGrpcServer() {
  const definition = protoLoader.loadSync(PROTO_PATH, {
    keepCase: false,
    longs: String,
    enums: String,
    defaults: false,
    oneofs: false,
    includeDirs: PROTO_INCLUDES,
  });
  const pkg = /** @type {any} */ (grpc.loadPackageDefinition(definition)).trogonatlas.api.eventmodel.v1alpha1;

  const server = new grpc.Server();
  server.addService(pkg.EventModelService.service, {
    getServerInfo: (_call, callback) => callback(null, { schemaVersion: 'v1', serverVersion: 'e2e-fixture' }),
    listEntities,
    getEntity,
    listEntitiesByDomain,
    listBranches,
    diffBranch,
    listValidationRules,
    listEntityKinds,
    searchEntities,
    validateEventModel,
    validateProject,
    listChanges,
    deleteByQuery,
    compileTypeLibrary,
    getTypeLibraryDescriptorSet,
    createBranch: unimplemented,
    deleteBranch: unimplemented,
    mergeBranch: unimplemented,
    updateBranch: unimplemented,
    resolveBranchEntry: unimplemented,
    inferDataFlow: unimplemented,
    checkInformationCompleteness: unimplemented,
  });

  const port = await new Promise((resolve, reject) => {
    server.bindAsync('127.0.0.1:0', grpc.ServerCredentials.createInsecure(), (err, boundPort) => {
      if (err) reject(err);
      else resolve(boundPort);
    });
  });

  return {
    url: `http://127.0.0.1:${port}`,
    stop: () =>
      new Promise((resolve) => {
        server.tryShutdown(() => resolve());
      }),
  };
}

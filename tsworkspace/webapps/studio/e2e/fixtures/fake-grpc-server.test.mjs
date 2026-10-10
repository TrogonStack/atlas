/**
 * Wire-level contract tests for the e2e fake gRPC backend.
 * Responses must survive a real protobuf encode/decode round-trip with
 * the same proto-loader options the studio bridge uses; wrong field
 * names are silently stripped on the wire (defaults:false).
 */
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import grpc from '@grpc/grpc-js';
import protoLoader from '@grpc/proto-loader';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  ORDER_MODEL_ID,
  startFakeGrpcServer,
} from './fake-grpc-server.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PROTO_PATH = path.resolve(__dirname, '../../../../../proto/trogonatlas/api/eventmodel/v1alpha1/service.proto');
const PROTO_INCLUDES = [path.resolve(__dirname, '../../../../../proto')];

/** @type {{ url: string; stop: () => Promise<void> }} */
let server;
/** @type {any} */
let client;

function call(method, request = {}) {
  return new Promise((resolve, reject) => {
    client[method](request, new grpc.Metadata(), { deadline: new Date(Date.now() + 5_000) }, (err, res) => {
      if (err) reject(err);
      else resolve(res);
    });
  });
}

beforeAll(async () => {
  server = await startFakeGrpcServer();
  const definition = protoLoader.loadSync(PROTO_PATH, {
    keepCase: false,
    longs: String,
    enums: String,
    defaults: false,
    oneofs: false,
    includeDirs: PROTO_INCLUDES,
  });
  const pkg = /** @type {any} */ (grpc.loadPackageDefinition(definition)).trogonatlas.api.eventmodel.v1alpha1;
  client = new pkg.EventModelService(
    server.url.replace(/^https?:\/\//, ''),
    grpc.credentials.createInsecure(),
  );
});

afterAll(async () => {
  await server.stop();
});

describe('fake-grpc ListEntities namespace filter', () => {
  it('returns no entities when namespaces does not match the fixture', async () => {
    const res = await call('listEntities', {
      namespaces: ['does-not-exist'],
      latestVersionsOnly: true,
    });
    expect(res.entities ?? []).toEqual([]);
  });

  it('returns the fixture EventModel when namespaces matches shop', async () => {
    const res = await call('listEntities', {
      namespaces: ['shop'],
      latestVersionsOnly: true,
    });
    expect(res.entities?.length).toBe(1);
    expect(res.entities[0].eventModel?.id?.namespace).toBe('shop');
  });
});

describe('fake-grpc ValidateEventModel response shape', () => {
  it('preserves issues under the proto field name `issues` (not `violations`)', async () => {
    // Dedicated fixture slug so the default order-service path stays
    // issue-free for smoke. Non-empty is required: defaults:false omits
    // empty repeated fields, which would hide a wrong field name.
    const res = await call('validateEventModel', {
      eventModelId: { namespace: 'shop', slug: 'with-validation-issue', version: '1' },
    });
    expect(res.issues?.length).toBeGreaterThan(0);
    expect(res.issues[0].code).toBe('E2E_SENTINEL');
    expect(res).not.toHaveProperty('violations');
  });
});

describe('fake-grpc DeleteByQuery response shape', () => {
  it('returns deleted as a repeated list (not a bare number)', async () => {
    // Non-empty delete result so the field survives defaults:false.
    const res = await call('deleteByQuery', {
      namespace: 'shop',
      slug: 'order.placed',
      maxDeletes: 1,
    });
    expect(Array.isArray(res.deleted)).toBe(true);
    expect(res.deleted.length).toBeGreaterThan(0);
    expect(res.deletedCount).toBe(1);
    expect(typeof res.deleted).not.toBe('number');
  });
});

describe('fake-grpc happy-path smoke RPCs', () => {
  it('GetEntity returns the order EventModel', async () => {
    const res = await call('getEntity', { kind: 12, id: ORDER_MODEL_ID });
    expect(res.entity?.eventModel?.title).toBe('Order Service');
  });
});

describe('fake-grpc type library RPCs', () => {
  it('CompileTypeLibrary returns positioned diagnostics', async () => {
    const res = await call('compileTypeLibrary', {
      library: {
        id: { namespace: 'shop', slug: 'shop.orders.v1', version: '1' },
        files: [
          { path: 'shop/orders/v1/ok.proto', content: 'syntax = "proto3";' },
          { path: 'shop/orders/v1/bad.proto', content: 'message X {}' },
        ],
      },
    });
    expect(res.diagnostics).toEqual([
      { path: 'shop/orders/v1/bad.proto', line: 1, column: 1, message: 'expected syntax declaration' },
    ]);
  });

  it('GetTypeLibraryDescriptorSet lists message names under `messages`', async () => {
    const res = await call('getTypeLibraryDescriptorSet', { namespace: 'shop' });
    expect(res.messages?.map((m) => m.fullName)).toEqual(['shop.orders.v1.OrderPlaced']);
  });
});

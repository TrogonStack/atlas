import path from 'node:path';
import { fileURLToPath } from 'node:url';
import protobuf from 'protobufjs';
import { beforeAll, describe, expect, it } from 'vitest';

const here = path.dirname(fileURLToPath(import.meta.url));
const protoRoot = path.resolve(here, '..', '..', '..', '..', 'proto');
const PKG = 'trogonatlas.eventmodel.v1alpha1';

let decodeAnnotations;
let root;

beforeAll(async () => {
  process.env.NODE_ENV = 'test';
  ({ decodeAnnotations } = await import('./index.mjs'));
  root = new protobuf.Root();
  root.resolvePath = (_origin, target) => path.resolve(protoRoot, target);
  await root.load(path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'));
});

function encode(name, payload) {
  const type = root.lookupType(`${PKG}.${name}`);
  return Buffer.from(type.encode(type.fromObject(payload)).finish());
}

describe('decodeAnnotations', () => {
  it('decodes Any annotations nested inside an Any-wrapped schema', () => {
    const annotation = encode('PersonalDataAnnotation', { doc: 'customer email' });
    const schema = encode('Schema', {
      title: 'Customer',
      fields: [
        {
          name: 'customer_id',
          metadata: [{ type_url: `type.googleapis.com/${PKG}.PersonalDataAnnotation`, value: annotation }],
        },
      ],
    });

    const [decoded] = decodeAnnotations([
      { event: { schema: { typeUrl: `type.googleapis.com/${PKG}.Schema`, value: schema } } },
    ]);

    expect(decoded.event.schema.fields[0].metadata[0]).toMatchObject({
      '@type': `type.googleapis.com/${PKG}.PersonalDataAnnotation`,
      doc: 'customer email',
    });
  });
});

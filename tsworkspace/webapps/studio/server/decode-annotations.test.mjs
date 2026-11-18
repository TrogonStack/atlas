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
  await root.load([
    path.join(protoRoot, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'),
    path.join(protoRoot, 'trogonatlas/annotation/v1alpha1/annotations.proto'),
  ]);
});

function encode(name, payload) {
  const type = root.lookupType(name.includes('.') ? name : `${PKG}.${name}`);
  return Buffer.from(type.encode(type.fromObject(payload)).finish());
}

describe('decodeAnnotations', () => {
  it('preserves packed native metadata, optional presence, ordered identities and uint64 versions', () => {
    const generic = 'trogonatlas.annotation.v1alpha1';
    const payloads = [
      [`${PKG}.AgentAnnotation`, {
        model: { provider: { value: 'Example' }, version: { value: 'opaque-version' } },
        tools: [{ name: { value: 'lookup' }, type: { value: 'http' }, description: { value: 'Lookup' } }],
      }],
      [`${generic}.LifecycleAnnotation`, { status: 'draft', draft: { message: { value: 'Not ready' } } }],
      [`${generic}.BadgesAnnotation`, { badges: [{ content: { value: 'Preview' }, backgroundColor: { value: '#aAbBcC' }, textColor: { value: '#001122' } }] }],
      [`${generic}.ResourceGroupsAnnotation`, { groups: [{
        groupId: { value: '01900000-0000-7000-8000-000000000001' },
        sidebar: false,
        displayLimit: { value: 1 },
        items: ['01900000-0000-7000-8000-000000000002', '01900000-0000-7000-8000-000000000003'].map((value) => ({
          connectionId: { value },
          target: { kind: 'ENTITY_KIND_COMPONENT', id: { namespace: 'metadata', slug: 'target', version: '18446744073709551615' } },
        })),
      }] }],
    ];
    const component = root.lookupType(`${PKG}.Component`);
    const bytes = component.encode(component.fromObject({
      id: { namespace: 'metadata', slug: 'worker', version: '1' },
      title: 'Worker',
      metadata: payloads.map(([name, payload]) => ({ type_url: `type.googleapis.com/${name}`, value: encode(name, payload) })),
    })).finish();
    const wire = component.toObject(component.decode(bytes), { longs: String, enums: String });
    const decoded = decodeAnnotations({ component: wire }).component;
    expect(decoded.metadata.map((value) => value['@type'])).toEqual(payloads.map(([name]) => `type.googleapis.com/${name}`));
    expect(decoded.metadata[0].model).toEqual({ provider: { value: 'Example' }, version: { value: 'opaque-version' } });
    expect(decoded.metadata[0].tools[0].description).toEqual({ value: 'Lookup' });
    expect(decoded.metadata[1].draft).toEqual({ message: { value: 'Not ready' } });
    expect(decoded.metadata[2].badges[0].backgroundColor.value).toBe('#aAbBcC');
    const group = decoded.metadata[3].groups[0];
    expect(group.sidebar).toBe(false);
    expect(group.displayLimit.value).toBe(1);
    expect(group.items.map((item) => item.connectionId.value)).toEqual(['01900000-0000-7000-8000-000000000002', '01900000-0000-7000-8000-000000000003']);
    expect(group.items.map((item) => item.target.id.version)).toEqual(['18446744073709551615', '18446744073709551615']);
  });

  it.each([
    `${PKG}.AgentAnnotation`,
    'trogonatlas.annotation.v1alpha1.LifecycleAnnotation',
    'trogonatlas.annotation.v1alpha1.BadgesAnnotation',
    'trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation',
  ])('keeps malformed known annotation %s visible as a decode failure', (name) => {
    const [decoded] = decodeAnnotations([{ typeUrl: `type.googleapis.com/${name}`, value: Buffer.from([0xff]) }]);
    expect(decoded).toMatchObject({ '@type': `type.googleapis.com/${name}`, '@decodeError': 'Malformed native annotation' });
    expect(decoded.value).toBeUndefined();
  });

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

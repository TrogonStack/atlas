// BUG proofs: Inspector Fields + Scenario UI residuals.
// Scenario example payloads accept Struct AND generated message Anys.
import { describe, expect, it } from 'vitest';
import type { WireEntityEnvelope } from './api';
import { buildModel } from './model';

describe('BUG: schema-only entities must surface fields for the Inspector', () => {
  it('reads FieldSpec list from a decoded trogonatlas.eventmodel.v1alpha1.Schema Any', () => {
    const wire: WireEntityEnvelope[] = [
      {
        event: {
          id: { namespace: 'orders', slug: 'placed', version: '1' },
          title: 'Order placed',
          schema: {
            '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema',
            title: 'OrderPlaced',
            fields: [
              { name: 'orderId', type: { uuid: { version: 7 } }, doc: 'stable id' },
              { name: 'total', type: { int: {} } },
            ],
          },
        },
      },
    ];
    const ev = buildModel(wire).entities.find((e) => e.kind === 'event');
    expect(ev?.fields.map((f) => f.name)).toEqual(['orderId', 'total']);
  });
});

describe('BUG: scenario example payloads must surface non-Struct Any values', () => {
  it('keeps Struct-packed example data (pre-codegen convention)', () => {
    const wire: WireEntityEnvelope[] = [
      {
        commandSlice: {
          id: { namespace: 'o', slug: 's-place', version: '1' },
          title: 'place',
          scenarios: [
            {
              id: 'happy',
              title: 'Happy',
              when: {
                command: { id: { namespace: 'o', slug: 'place', version: '1' } },
                payload: {
                  '@type': 'type.googleapis.com/google.protobuf.Struct',
                  value: { total: 42 },
                },
              },
              emit: {
                events: [{ event: { id: { namespace: 'o', slug: 'placed', version: '1' } } }],
              },
            },
          ],
        },
      },
    ];
    const sc = buildModel(wire).slices[0]?.scenarios[0];
    expect(sc?.when[0]?.data).toEqual({ total: 42 });
  });

  it('keeps generated-message Any payloads (fields beside @type)', () => {
    const wire: WireEntityEnvelope[] = [
      {
        commandSlice: {
          id: { namespace: 'o', slug: 's-place', version: '1' },
          title: 'place',
          scenarios: [
            {
              id: 'happy',
              title: 'Happy',
              when: {
                command: { id: { namespace: 'o', slug: 'place', version: '1' } },
                payload: {
                  '@type': 'type.googleapis.com/orders.v1.Place',
                  total: 42,
                  currency: 'USD',
                },
              },
              emit: {
                events: [{ event: { id: { namespace: 'o', slug: 'placed', version: '1' } } }],
              },
            },
          ],
        },
      },
    ];
    const sc = buildModel(wire).slices[0]?.scenarios[0];
    expect(sc?.when[0]?.data).toEqual({ total: 42, currency: 'USD' });
  });

  it('keeps already-unwrapped plain object payloads', () => {
    const wire: WireEntityEnvelope[] = [
      {
        commandSlice: {
          id: { namespace: 'o', slug: 's-place', version: '1' },
          title: 'place',
          scenarios: [
            {
              id: 'reject',
              title: 'Too low',
              when: {
                command: { id: { namespace: 'o', slug: 'place', version: '1' } },
                payload: { total: 7 },
              },
              reject: { reasonCode: 'TOO_LOW', doc: 'minimum not met' },
            },
          ],
        },
      },
    ];
    const sc = buildModel(wire).slices[0]?.scenarios[0];
    expect(sc?.when[0]?.data).toEqual({ total: 7 });
    expect(sc?.reject).toEqual({ reasonCode: 'TOO_LOW', doc: 'minimum not met' });
  });
});

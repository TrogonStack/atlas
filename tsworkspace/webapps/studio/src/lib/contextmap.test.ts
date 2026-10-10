import { describe, expect, it } from 'vitest';
import { buildContextMap } from './contextmap';
import { buildModel } from './model';

const id = (namespace: string, slug: string): Record<string, unknown> => ({
  namespace,
  slug,
  version: '1',
});

function fixtureEntities(): Record<string, unknown>[] {
  return [
    {
      boundedContext: {
        id: id('orders', 'orders'),
        title: 'Orders',
      },
    },
    {
      boundedContext: {
        id: id('billing', 'billing'),
        title: 'Billing',
        relationships: [
          {
            upstream: { id: id('orders', 'orders') },
            intent: 'INTENT_CUSTOMER_SUPPLIER',
            doc: 'billing follows placed orders',
          },
        ],
      },
    },
    { event: { id: id('orders', 'order.placed'), title: 'Order placed' } },
    {
      readModel: {
        id: id('billing', 'orders-to-bill'),
        title: 'Orders to bill',
        sourceEvents: [{ id: id('orders', 'order.placed') }],
      },
    },
  ];
}

describe('buildContextMap', () => {
  it('derives seam edges between bounded contexts from subscription views', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes, edges } = buildContextMap(model);

    expect(nodes.map((n) => n.id).sort()).toEqual(['ctx:billing', 'ctx:orders']);
    expect(edges).toHaveLength(1);
    expect(edges[0]).toMatchObject({
      source: 'ctx:orders',
      target: 'ctx:billing',
      label: '1 view',
    });
    expect((edges[0].data as { views?: string[]; events?: string[]; intent?: string }).views).toEqual([
      'Orders to bill',
    ]);
    expect((edges[0].data as { views?: string[]; events?: string[]; intent?: string }).events).toEqual([
      'order.placed',
    ]);
    expect((edges[0].data as { intent?: string }).intent).toBe('customer supplier');
  });

  it('still renders contexts when no seam exists yet', () => {
    const model = buildModel(fixtureEntities().slice(0, 2) as never);
    const { nodes, edges } = buildContextMap(model);
    expect(nodes).toHaveLength(2);
    expect(edges).toHaveLength(0);
  });

  // BUG: relationshipsOf().find uses `namespace === OR slug ===`, so a
  // same-slug upstream in a different namespace steals the seam intent.
  it('attributes the relationship intent for the matching upstream namespace (not slug alone)', () => {
    const model = buildModel([
      {
        boundedContext: {
          id: id('orders', 'orders'),
          title: 'Orders',
        },
      },
      {
        boundedContext: {
          id: id('payments', 'orders'),
          title: 'Payments',
        },
      },
      {
        boundedContext: {
          id: id('billing', 'billing'),
          title: 'Billing',
          relationships: [
            {
              upstream: { id: id('payments', 'orders') },
              intent: 'INTENT_CONFORMIST',
              doc: 'billing mirrors payments',
            },
            {
              upstream: { id: id('orders', 'orders') },
              intent: 'INTENT_CUSTOMER_SUPPLIER',
              doc: 'billing follows placed orders',
            },
          ],
        },
      },
      { event: { id: id('orders', 'order.placed'), title: 'Order placed' } },
      {
        readModel: {
          id: id('billing', 'orders-to-bill'),
          title: 'Orders to bill',
          sourceEvents: [{ id: id('orders', 'order.placed') }],
        },
      },
    ] as never);

    const { edges } = buildContextMap(model);
    const seam = edges.find((e) => e.source === 'ctx:orders' && e.target === 'ctx:billing');
    expect(seam).toBeDefined();
    expect((seam?.data as { intent?: string }).intent).toBe('customer supplier');
  });
});

import { describe, expect, it } from 'vitest';
import type { NamespaceRegistryRow } from './api';
import { DEFAULT_OWNER, groupByOwner, ownerLabel } from './namespace-owner';

function row(overrides: Partial<NamespaceRegistryRow> & { name: string }): NamespaceRegistryRow {
  return { id: overrides.name, parent: '', entityCount: 1, registered: true, ...overrides };
}

const model = (namespace: string, slug: string) => ({ namespace, slug });
const shape = (groups: ReturnType<typeof groupByOwner<{ namespace: string; slug: string }>>) =>
  groups.map((g) => ({
    owner: g.owner,
    namespaces: g.namespaces.map((n) => [n.namespace, n.items.map((m) => m.slug)]),
  }));

describe('ownerLabel', () => {
  it('names the server default when a row has no parent', () => {
    expect(ownerLabel(row({ name: 'acme-orders' }))).toBe(DEFAULT_OWNER);
    expect(ownerLabel(row({ name: 'acme-orders', parent: 'acme' }))).toBe('acme');
  });
});

describe('groupByOwner', () => {
  const registry = [
    row({ name: 'acme-orders', parent: 'acme' }),
    row({ name: 'acme-probe', parent: 'acme' }),
    row({ name: 'other-ns' }),
  ];

  it('puts namespaces that share a parent under one owner', () => {
    const items = [
      model('other-ns', 'x'),
      model('acme-probe', 'p'),
      model('acme-orders', 'b'),
      model('acme-orders', 'a'),
    ];
    expect(shape(groupByOwner(items, (m) => m.namespace, registry))).toEqual([
      {
        owner: 'acme',
        namespaces: [
          ['acme-orders', ['b', 'a']],
          ['acme-probe', ['p']],
        ],
      },
      { owner: DEFAULT_OWNER, namespaces: [['other-ns', ['x']]] },
    ]);
  });

  it('matches a minted id, since entity keys carry the id and not the name', () => {
    const minted = [row({ id: 'ns_01', name: 'acme-billing', parent: 'acme' })];
    expect(shape(groupByOwner([model('ns_01', 'a')], (m) => m.namespace, minted))).toEqual([
      { owner: 'acme', namespaces: [['ns_01', ['a']]] },
    ]);
  });

  it('lists namespaces without an owner last, in a group no owner name can claim', () => {
    const rows = [...registry, row({ id: '', name: 'legacy', registered: false })];
    const items = [model('legacy', 'l'), model('missing', 'm'), model('acme-orders', 'a')];
    expect(shape(groupByOwner(items, (m) => m.namespace, rows))).toEqual([
      { owner: 'acme', namespaces: [['acme-orders', ['a']]] },
      {
        owner: null,
        namespaces: [
          ['legacy', ['l']],
          ['missing', ['m']],
        ],
      },
    ]);
  });

  it('keeps an owner literally named unregistered apart from the no-owner group', () => {
    const rows = [
      row({ name: 'acme-orders', parent: 'unregistered' }),
      row({ id: '', name: 'legacy', registered: false }),
    ];
    const items = [model('legacy', 'l'), model('acme-orders', 'a')];
    expect(shape(groupByOwner(items, (m) => m.namespace, rows))).toEqual([
      { owner: 'unregistered', namespaces: [['acme-orders', ['a']]] },
      { owner: null, namespaces: [['legacy', ['l']]] },
    ]);
  });

  it('keeps one ownerless group when the registry could not be read', () => {
    const items = [model('b-ns', 'b'), model('a-ns', 'a')];
    expect(shape(groupByOwner(items, (m) => m.namespace, undefined))).toEqual([
      {
        owner: null,
        namespaces: [
          ['a-ns', ['a']],
          ['b-ns', ['b']],
        ],
      },
    ]);
  });
});

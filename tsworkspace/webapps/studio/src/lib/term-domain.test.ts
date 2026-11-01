import { expect, it } from 'vitest';
import { buildModel, termOwningDomain } from './model';

it('retains an exact declared domain independently of the term namespace', () => {
  const model = buildModel([
    {
      term: {
        id: { namespace: 'context', slug: 'word', version: '1' },
        owningDomain: { id: { namespace: 'business', slug: 'sales', version: '7' } },
      },
    },
  ]);
  expect(termOwningDomain(model.entities[0])).toEqual({ namespace: 'business', slug: 'sales', version: '7' });
});

it('does not infer ownership from namespace or accept an unversioned selection', () => {
  for (const owningDomain of [undefined, { id: { namespace: 'business', slug: 'sales', version: '0' } }]) {
    const model = buildModel([{ term: { id: { namespace: 'context', slug: 'word', version: '1' }, owningDomain } }]);
    expect(termOwningDomain(model.entities[0])).toBeUndefined();
  }
});

it.each(['01', '18446744073709551616', 9007199254740992, '1.0', '-1'])(
  'rejects a noncanonical or inexact domain version %s',
  (version) => {
    const model = buildModel([
      {
        term: {
          id: { namespace: 'context', slug: 'word', version: '1' },
          owningDomain: { id: { namespace: 'business', slug: 'sales', version } },
        },
      },
    ]);
    expect(termOwningDomain(model.entities[0])).toBeUndefined();
  },
);

it.each(['18446744073709551615', 7])('preserves exact domain version %s', (version) => {
  const model = buildModel([
    {
      term: {
        id: { namespace: 'context', slug: 'word', version: '1' },
        owningDomain: { id: { namespace: 'business', slug: 'sales', version } },
      },
    },
  ]);
  expect(termOwningDomain(model.entities[0])?.version).toBe(String(version));
});

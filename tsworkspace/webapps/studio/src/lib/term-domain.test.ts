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

// Branch review badges on sticky cards (Phase 3): every diff status the
// board can stamp must render its chip; entities without a status render
// no chip; unknown statuses are ignored rather than crashing the card.
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { buildModel } from '@/lib/model';
import { StickyCardBody } from './StickyNode';

function entity() {
  const model = buildModel([
    {
      event: {
        id: { namespace: 'shop', slug: 'order.placed', version: '1' },
        title: 'Order placed',
      },
    },
  ] as never);
  const found = model.entities.find((e) => e.kind === 'event');
  if (!found) throw new Error('fixture entity missing');
  return found;
}

describe('StickyCardBody branch badges', () => {
  afterEach(cleanup);

  it.each(['added', 'changed', 'deleted', 'conflict'] as const)('renders the %s badge', (status) => {
    render(<StickyCardBody entity={entity()} branchStatus={status} />);
    expect(screen.getAllByText(status).length).toBeGreaterThan(0);
  });

  it('renders no badge without a branch status', () => {
    render(<StickyCardBody entity={entity()} />);
    for (const label of ['added', 'changed', 'deleted', 'conflict']) {
      expect(screen.queryAllByText(label).length).toBe(0);
    }
  });

  it('ignores unknown statuses instead of crashing', () => {
    render(<StickyCardBody entity={entity()} branchStatus="converged" />);
    expect(screen.getAllByText('Order placed').length).toBeGreaterThan(0);
  });
});

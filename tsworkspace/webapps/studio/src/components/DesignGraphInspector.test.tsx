import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from '@/lib/api';
import { buildModel } from '@/lib/model';
import { DesignGraphInspector } from './DesignGraphInspector';

const id = { namespace: 'catalog', slug: 'checkout-flow', version: '7' };
const subject = { entity: { kind: 'ENTITY_KIND_COMPONENT', id: { ...id, slug: 'checkout' } } };
function graph(kind = 'designGraph') {
  return buildModel([
    {
      [kind]: {
        id,
        title: { value: 'Checkout flow' },
        profile: 'DESIGN_GRAPH_PROFILE_FLOW',
        subjects: [{ subject, connectionId: { value: 'subject-b' } }],
        steps: [{ stepId: { value: 'step-b' }, title: { value: 'Pay' } }],
      },
      system: { uid: 'graph-incarnation' },
    },
  ]).entities[0];
}
beforeEach(() => {
  for (const method of [
    'getDesignGraphDetail',
    'listSubjectDesignGraphs',
    'traverseDesignGraphTopology',
    'listIncomingDesignGraphSubflows',
  ] as const)
    vi.spyOn(api, method).mockResolvedValue({
      scope: { branch: 'review', revision: 'checked-revision' },
      incomplete: true,
    });
  window.history.replaceState({}, '', '/?branch=review');
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.history.replaceState({}, '', '/');
});
describe('design topology inspector', () => {
  it('recognizes the kind and title and issues exact branch-scoped read queries', async () => {
    const entity = graph();
    expect(entity.kind).toBe('designGraph');
    expect(entity.title).toBe('Checkout flow');
    render(<DesignGraphInspector entity={entity} />);
    await waitFor(() => expect(api.getDesignGraphDetail).toHaveBeenCalled());
    const branch = expect.objectContaining({ branch: 'review', signal: expect.any(AbortSignal) });
    expect(api.getDesignGraphDetail).toHaveBeenCalledWith({ id }, branch);
    expect(api.traverseDesignGraphTopology).toHaveBeenCalledWith({ id }, { followSubflows: true }, branch);
    expect(api.listIncomingDesignGraphSubflows).toHaveBeenCalledWith({ id }, branch);
    expect(api.listSubjectDesignGraphs).toHaveBeenCalledWith(subject, branch);
    expect((await screen.findAllByText(/Incomplete inspection/)).length).toBeGreaterThan(0);
  });
  it('suspends hydration for history and renders nothing for other kinds', () => {
    const { container, rerender } = render(<DesignGraphInspector entity={graph()} asOf="retained-revision" />);
    expect(api.getDesignGraphDetail).not.toHaveBeenCalled();
    expect(container.textContent).toMatch(/Historical topology hydration is unavailable/);
    rerender(<DesignGraphInspector entity={graph('decision')} />);
    expect(container.textContent).toBe('');
  });
});

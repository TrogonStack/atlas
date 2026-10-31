import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
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
  it('shows hydrated topology without treating absent declaration fields as empty catalog pages', async () => {
    const scope = { branch: 'review', revision: 'checked-revision' };
    const owner = { owner: { kind: 'ENTITY_KIND_DESIGN_GRAPH', id }, uid: 'graph-incarnation' };
    const detail = {
      entity: { designGraph: { id, title: { value: 'Hydrated checkout flow' } } },
      targets: [{ stepId: { value: 'resolved-step' }, state: 'CATALOG_RESOLUTION_STATE_RESOLVED' }],
      scope,
      incomplete: false,
    };
    const traversal = {
      nodes: [{ graph: owner, stepId: { value: 'traversed-step' } }],
      edges: [{ graph: owner, fromStepId: { value: 'traversed-step' }, toStepId: { value: 'finish-step' } }],
      scope,
      incomplete: false,
    };
    const subflows = {
      references: [{ owner, field: 'steps[0].subflow', state: 'CATALOG_RESOLUTION_STATE_RESOLVED' }],
      scope,
      incomplete: false,
    };
    vi.mocked(api.getDesignGraphDetail).mockResolvedValue(detail);
    vi.mocked(api.traverseDesignGraphTopology).mockResolvedValue(traversal);
    vi.mocked(api.listIncomingDesignGraphSubflows).mockResolvedValue(subflows);
    vi.mocked(api.listSubjectDesignGraphs).mockResolvedValue({ scope, declarations: [], incomplete: false });
    render(<DesignGraphInspector entity={graph()} />);
    for (const [title, marker] of [
      ['Design graph detail', /resolved-step/],
      ['Topology traversal', /traversed-step/],
      ['Incoming subflow references', /steps\[0\]\.subflow/],
    ] as const) {
      const section = screen.getByRole('heading', { name: title }).closest('section');
      expect(section).not.toBeNull();
      const query = within(section!);
      expect(await query.findByText(marker)).toBeTruthy();
      expect(query.queryByText(/No declarations returned/)).toBeNull();
    }
    const subjectQuery = screen.getByRole('heading', { name: 'Design graphs for exact subject' }).closest('section');
    expect(subjectQuery).not.toBeNull();
    expect(await within(subjectQuery!).findByText(/No declarations returned/)).toBeTruthy();
  });
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

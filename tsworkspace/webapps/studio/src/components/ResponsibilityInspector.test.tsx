import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from '@/lib/api';
import { buildModel } from '@/lib/model';
import type { ResponsibilityAssignments } from '@/lib/responsibility';
import { ResponsibilityInspector } from './ResponsibilityInspector';
import { Inspector } from './Inspector';

vi.mock('@/lib/useEntityHomes', () => ({
  useEntityHomes: () => ({ homes: new Map(), error: undefined, retry: vi.fn() }),
}));

const id = { namespace: 'catalog', slug: 'team', version: '3' };
const port = {
  port: {
    product: { id: { ...id, slug: 'offering', version: '9' } },
    ownerUid: 'product-incarnation',
    direction: 'DATA_PORT_DIRECTION_OUTPUT',
    portId: { value: 'output-b' },
  },
} as const;
const scope = {
  branch: 'review',
  revision: 'checked-revision',
  authorizedNamespaces: ['catalog'],
  visitedSubjects: [port],
};
function entity(kind = 'team') {
  return buildModel([
    {
      [kind]: {
        id,
        title: { value: 'Catalog team' },
        contacts: [{ email: { value: 'team@example.test' } }],
        responsibilities: [
          { connectionId: { value: 'retained-assignment' }, kind: 'RESPONSIBILITY_KIND_STEWARD', subject: port },
        ],
      },
      system: { uid: 'team-incarnation' },
    },
  ]).entities[0];
}
beforeEach(() => {
  vi.spyOn(api, 'listSubjectAssignments').mockResolvedValue({
    scope,
    assignments: [],
    subjectState: 'RESPONSIBILITY_RESOLUTION_STATE_RESOLVED',
  });
  vi.spyOn(api, 'listAssigneeResponsibilities').mockResolvedValue({
    scope,
    incomplete: true,
    assignments: [
      {
        assignee: { team: { id } },
        assigneeUid: 'team-incarnation',
        assignment: {
          connectionId: { value: 'retained-assignment' },
          kind: 'RESPONSIBILITY_KIND_STEWARD',
          subject: port,
        },
        state: 'RESPONSIBILITY_RESOLUTION_STATE_UNRESOLVED',
      },
    ],
  });
  vi.spyOn(api, 'listTeamMembers').mockResolvedValue({
    scope,
    memberships: [
      {
        team: { id },
        teamUid: 'team-incarnation',
        membership: { connectionId: { value: 'membership-a' }, person: { id: { ...id, slug: 'person' } } },
        state: 'RESPONSIBILITY_RESOLUTION_STATE_INACCESSIBLE',
      },
    ],
  });
  vi.spyOn(api, 'listPersonTeams').mockResolvedValue({ scope, memberships: [] });
  vi.spyOn(api, 'getTeamContacts').mockResolvedValue({
    scope,
    teamContacts: [
      {
        team: { id },
        teamUid: 'team-incarnation',
        contact: { email: { value: 'team@example.test' } },
        state: 'RESPONSIBILITY_RESOLUTION_STATE_RESOLVED',
      },
    ],
    memberContacts: [],
  });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.history.replaceState({}, '', '/');
});

describe('backend responsibility inspection', () => {
  it('preserves selectors, checked context and independent unresolved/inaccessible/incomplete evidence', async () => {
    window.history.replaceState({}, '', '/?branch=review');
    render(<ResponsibilityInspector entity={entity()} />);
    await screen.findByText('Backend assignment state: unresolved');
    expect(screen.getByText('Backend membership state: inaccessible')).toBeDefined();
    expect(screen.getByText(/Incomplete backend inspection/)).toBeDefined();
    expect(screen.getAllByText(/checked-revision/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/product-incarnation/).length).toBeGreaterThan(0);
    expect(api.listSubjectAssignments).toHaveBeenCalledWith(
      { subject: { entity: { kind: 'ENTITY_KIND_TEAM', id } }, kind: undefined },
      expect.objectContaining({ branch: 'review' }),
    );
    expect(api.getTeamContacts).toHaveBeenCalledWith({ id }, false, expect.objectContaining({ branch: 'review' }));
    fireEvent.change(screen.getByLabelText('Exact responsibility subject'), {
      target: { value: JSON.stringify(port) },
    });
    fireEvent.change(screen.getByLabelText('Responsibility kind'), {
      target: { value: 'RESPONSIBILITY_KIND_CONTACT' },
    });
    await waitFor(() =>
      expect(api.listSubjectAssignments).toHaveBeenLastCalledWith(
        { subject: port, kind: 'RESPONSIBILITY_KIND_CONTACT' },
        expect.objectContaining({ branch: 'review' }),
      ),
    );
    vi.mocked(api.getTeamContacts).mockResolvedValueOnce({
      scope,
      teamContacts: [],
      memberContacts: [
        {
          team: { id },
          teamUid: 'team-incarnation',
          membershipId: { value: 'member-contact-provenance' },
          person: { id: { ...id, slug: 'member' } },
          contact: { chat: { service: { value: 'chat' }, handle: { value: 'member-handle' } } },
          state: 'RESPONSIBILITY_RESOLUTION_STATE_RESOLVED',
        },
      ],
    });
    fireEvent.click(screen.getByLabelText('Include authorized member contacts'));
    await waitFor(() =>
      expect(api.getTeamContacts).toHaveBeenLastCalledWith({ id }, true, expect.objectContaining({ branch: 'review' })),
    );
    await screen.findByText(/member-contact-provenance/);
    expect(screen.getByText('Authored Team contacts')).toBeDefined();
    expect(screen.getByText('Expanded member contacts with membership provenance')).toBeDefined();
  });
  it.each([
    ['RESPONSIBILITY_RESOLUTION_STATE_INACCESSIBLE', 'Subject inaccessible; direct assignments remain unknown.'],
    [
      'RESPONSIBILITY_RESOLUTION_STATE_UNRESOLVED',
      'Subject unresolved; the retained exact selector may be stale or removed.',
    ],
    [
      'RESPONSIBILITY_RESOLUTION_STATE_UNSPECIFIED',
      'No assignment records returned; complete visible absence remains unproven.',
    ],
  ] as const)('does not turn an empty %s response into confirmed absence', async (subjectState, message) => {
    vi.mocked(api.listSubjectAssignments).mockResolvedValue({ scope, subjectState, assignments: [] });
    render(<ResponsibilityInspector entity={entity()} />);
    await screen.findByText(message);
    expect(screen.queryByText(/No direct assignments returned/)).toBeNull();
  });

  it('keeps operational failures separate from visible empty and retained declarations', async () => {
    vi.mocked(api.listSubjectAssignments).mockRejectedValue(new Error('backend unavailable'));
    render(<ResponsibilityInspector entity={entity('person')} />);
    await screen.findByRole('alert');
    expect(screen.getByText(/backend unavailable/)).toBeDefined();
    expect(screen.getByText('Backend assignment state: unresolved')).toBeDefined();
    expect(api.listPersonTeams).toHaveBeenCalledWith({ id }, expect.anything());
    expect(api.getTeamContacts).not.toHaveBeenCalled();
    expect(screen.queryByText(/No direct assignments returned/)).toBeNull();
  });
  it('renders historical declarations without issuing any current query, including with a branch in the URL', () => {
    window.history.replaceState({}, '', '/?asOf=11111111-1111-4111-8111-111111111111&branch=review');
    render(<ResponsibilityInspector entity={entity()} />);
    expect(screen.getByText(/Historical responsibility inspection unavailable/)).toBeDefined();
    expect(screen.getByText(/11111111-1111-4111-8111-111111111111/)).toBeDefined();
    for (const method of [
      api.listSubjectAssignments,
      api.listAssigneeResponsibilities,
      api.listTeamMembers,
      api.listPersonTeams,
      api.getTeamContacts,
    ])
      expect(method).not.toHaveBeenCalled();
  });
  it('discards late responses after context changes and does not show another branch as checked', async () => {
    let resolve: (data: ResponsibilityAssignments) => void = () => {};
    vi.mocked(api.listSubjectAssignments).mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    window.history.replaceState({}, '', '/?branch=old');
    const rendered = render(<ResponsibilityInspector entity={entity()} />);
    const oldSignal = vi.mocked(api.listSubjectAssignments).mock.calls[0][1]?.signal;
    window.history.replaceState({}, '', '/?branch=new');
    rendered.rerender(<ResponsibilityInspector entity={entity()} />);
    await waitFor(() => expect(oldSignal?.aborted).toBe(true));
    resolve({ scope: { revision: 'stale-result' } });
    await waitFor(() =>
      expect(api.listSubjectAssignments).toHaveBeenLastCalledWith(
        expect.anything(),
        expect.objectContaining({ branch: 'new' }),
      ),
    );
    expect(screen.queryByText(/stale-result/)).toBeNull();
  });
});

it('keeps the integrated Inspector historical while the URL still names a current branch', () => {
  window.history.replaceState({}, '', '/?branch=review');
  const team = entity();
  const model = buildModel([{ team: team.raw, system: { uid: team.uid } }]);
  render(
    <Inspector
      entity={team}
      model={model}
      asOf="11111111-1111-4111-8111-111111111111"
      onClose={vi.fn()}
      onSelect={vi.fn()}
    />,
  );
  expect(screen.getByText(/Historical responsibility inspection unavailable/)).toBeDefined();
  for (const method of [
    api.listSubjectAssignments,
    api.listAssigneeResponsibilities,
    api.listTeamMembers,
    api.listPersonTeams,
    api.getTeamContacts,
  ])
    expect(method).not.toHaveBeenCalled();
});

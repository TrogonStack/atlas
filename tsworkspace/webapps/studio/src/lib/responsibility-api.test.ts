import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getTeamContacts,
  listAssigneeResponsibilities,
  listPersonTeams,
  listSubjectAssignments,
  listTeamMembers,
} from './api';

const id = { namespace: 'catalog', slug: 'directory', version: '9007199254740993' };
const subject = {
  member: {
    specification: { id },
    ownerUid: 'exact-owner',
    kind: 'CONTRACT_MEMBER_KIND_ARTIFACT',
    memberId: { value: 'artifact-b' },
    expectedDigest: { sha256: 'a'.repeat(64) },
  },
};
afterEach(() => vi.unstubAllGlobals());
describe('typed responsibility HTTP client', () => {
  it('preserves exact selectors, filters, branch context and checked evidence over read-only HTTP', async () => {
    const response = { scope: { revision: 'checked', visitedSubjects: [subject] }, incomplete: true };
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify(response), { status: 200 }));
    vi.stubGlobal('fetch', fetch);
    await expect(
      listSubjectAssignments({ subject, kind: 'RESPONSIBILITY_KIND_CONTACT' }, { branch: 'review' }),
    ).resolves.toEqual(response);
    const [url, options] = fetch.mock.calls[0];
    const parsed = new URL(url, 'http://localhost');
    expect(parsed.pathname).toBe('/api/responsibility/subject-assignments');
    expect(JSON.parse(parsed.searchParams.get('request') ?? '')).toEqual({
      subject,
      kind: 'RESPONSIBILITY_KIND_CONTACT',
    });
    expect(parsed.searchParams.get('branch')).toBe('review');
    expect(options.headers['x-trogon-atlas-branch']).toBe('review');
    expect(options.method).toBeUndefined();
  });
  it('uses separate assignee, membership and contact queries, defaulting expansion to false', async () => {
    const fetch = vi.fn().mockImplementation(() => Promise.resolve(new Response('{}', { status: 200 })));
    vi.stubGlobal('fetch', fetch);
    await listAssigneeResponsibilities({ assignee: { team: { id } } });
    await listTeamMembers({ id });
    await listPersonTeams({ id });
    await getTeamContacts({ id });
    const requests = fetch.mock.calls.map(([url]) => {
      const parsed = new URL(url, 'http://localhost');
      return { path: parsed.pathname, input: JSON.parse(parsed.searchParams.get('request') ?? '') };
    });
    expect(requests).toEqual([
      { path: '/api/responsibility/assignee-responsibilities', input: { assignee: { team: { id } } } },
      { path: '/api/responsibility/team-members', input: { team: { id } } },
      { path: '/api/responsibility/person-teams', input: { person: { id } } },
      { path: '/api/responsibility/team-contacts', input: { team: { id }, includeMembers: false } },
    ]);
  });
  it('propagates backend failure instead of manufacturing empty assignments', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: 'budget exhausted' }), { status: 429 })),
    );
    await expect(listSubjectAssignments({ subject })).rejects.toThrow('budget exhausted');
  });
});

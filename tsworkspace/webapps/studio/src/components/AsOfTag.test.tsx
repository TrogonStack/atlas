import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/api')>();
  return { ...actual, api: { ...actual.api, changeset: vi.fn() } };
});

import { api } from '@/lib/api';
import { TopBar } from './TopBar';

const changeset = vi.mocked(api.changeset);
const AS_OF = '0192f000-0000-7000-8000-000000000003';
const AT = '2026-01-03T15:04:00Z';

beforeEach(() => {
  changeset.mockReset();
  window.history.replaceState({}, '', `/em/acme/checkout?asOf=${AS_OF}&view=sequence`);
});

afterEach(() => {
  cleanup();
  window.history.replaceState({}, '', '/');
});

function renderBar(onExitAsOf = vi.fn()) {
  render(
    <TopBar
      realtime={{ state: 'live' }}
      onRefresh={vi.fn()}
      branch={undefined}
      onSelectBranch={vi.fn()}
      onOpenHistory={vi.fn()}
      asOf={AS_OF}
      onExitAsOf={onExitAsOf}
    />,
  );
  return { onExitAsOf };
}

describe('TopBar as of a changeset', () => {
  it('shows an amber read-only tag with the changeset time in local time', async () => {
    changeset.mockResolvedValue({
      changeset: { id: AS_OF, author: 'acme-agent', at: AT, message: '', branch: '', rpc: 'PutEntity', ops: [] },
    });
    renderBar();

    const tag = screen.getByRole('status', { name: 'As of' });
    expect(tag.className).toContain('bg-amber-100');
    const when = new Date(AT).toLocaleString(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    });
    expect(await screen.findByText(`as of ${when}`)).toBeDefined();
    expect(tag.textContent).toContain('read only');
    expect(changeset).toHaveBeenCalledWith(AS_OF, expect.objectContaining({ signal: expect.anything() }));
  });

  it('hides the live, branch and menu controls but keeps History', () => {
    changeset.mockReturnValue(new Promise(() => {}));
    renderBar();
    expect(screen.queryByRole('status', { name: 'Live updates' })).toBeNull();
    expect(screen.queryByRole('button', { name: /^Branch:/ })).toBeNull();
    expect(screen.queryByRole('button', { name: 'More actions' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Namespace history' })).toBeDefined();
  });

  it('falls back to the changeset id when its time cannot be read', async () => {
    changeset.mockRejectedValue(new Error('unavailable'));
    renderBar();
    expect(await screen.findByText('as of changeset 0192f000')).toBeDefined();
  });

  it('exits through a link to the same board without asOf', () => {
    changeset.mockReturnValue(new Promise(() => {}));
    const { onExitAsOf } = renderBar();
    const exit = screen.getByRole('link', { name: 'Exit' });
    expect(exit.getAttribute('href')).toBe('/em/acme/checkout?view=sequence');
    fireEvent.click(exit);
    expect(onExitAsOf).toHaveBeenCalledTimes(1);
  });
});

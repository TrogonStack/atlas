import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { withNuqsTestingAdapter } from 'nuqs/adapters/testing';
import type { ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/api')>();
  return { ...actual, api: { ...actual.api, entityHistory: vi.fn() } };
});

vi.mock('@/lib/useEntityHomes', () => ({
  useEntityHomes: () => ({ homes: new Map(), error: undefined, retry: vi.fn() }),
}));

import { api, type EntityRevision } from '@/lib/api';
import { useBranchParam } from '@/lib/branch';
import { buildModel, type Entity } from '@/lib/model';
import { Inspector } from './Inspector';

const entityHistory = vi.mocked(api.entityHistory);

const id = { namespace: 'acme', slug: 'order.placed', version: '1' };

function revision(changesetId: string, overrides: Partial<EntityRevision> = {}): EntityRevision {
  return {
    changesetId,
    author: 'acme-agent',
    at: '2026-01-02T00:00:00Z',
    rpc: 'PutEntity',
    branch: '',
    kind: 'KIND_PUT',
    before: { event: { id, title: 'Order placed' } },
    after: { event: { id, title: 'Order placed by shopper' } },
    ...overrides,
  };
}

// Stands in for the branch bar: it flips `?branch=` while the Inspector stays mounted.
function WithBranchSwitch({ children }: { children: ReactNode }) {
  const [, setBranch] = useBranchParam();
  return (
    <>
      <button type="button" onClick={() => setBranch('alex/rework')}>
        switch branch
      </button>
      {children}
    </>
  );
}

function renderInspector() {
  const model = buildModel([{ event: { id, title: 'Order placed by shopper' } }] as never);
  const event = model.entities.find((e) => e.id.slug === 'order.placed') as Entity;
  render(
    <WithBranchSwitch>
      <Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />
    </WithBranchSwitch>,
    { wrapper: withNuqsTestingAdapter() },
  );
}

beforeEach(() => {
  entityHistory.mockReset();
});

afterEach(cleanup);

describe('Inspector: History tab', () => {
  it('fetches only once the tab is opened, then lists revisions and expands one to before/after', async () => {
    entityHistory.mockResolvedValue({
      revisions: [
        revision('0192f000-0000-7000-8000-00000000beef'),
        revision('0192f000-0000-7000-8000-00000000cafe', { before: null }),
      ],
      nextPageToken: '',
    });
    renderInspector();
    expect(entityHistory).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('tab', { name: 'History' }));
    expect(await screen.findByText('updated')).toBeDefined();
    expect(screen.getByText('created')).toBeDefined();
    expect(screen.getAllByText('acme-agent')).toHaveLength(2);
    expect(entityHistory).toHaveBeenCalledWith(
      { kind: 'event', namespace: 'acme', slug: 'order.placed', version: '1' },
      undefined,
      expect.anything(),
    );
    expect(screen.queryByRole('button', { name: 'Load older' })).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: /00000000beef/ }));
    expect(screen.getByText('Before')).toBeDefined();
    expect(screen.getByText(/"title": "Order placed by shopper"/)).toBeDefined();
  });

  it('appends the next page when Load older is clicked', async () => {
    entityHistory
      .mockResolvedValueOnce({ revisions: [revision('0192f000-0000-7000-8000-00000000beef')], nextPageToken: 'tok-1' })
      .mockResolvedValueOnce({
        revisions: [revision('0192f000-0000-7000-8000-00000000cafe', { before: null })],
        nextPageToken: '',
      });
    renderInspector();
    fireEvent.click(screen.getByRole('tab', { name: 'History' }));

    fireEvent.click(await screen.findByRole('button', { name: 'Load older' }));
    expect(await screen.findByText('created')).toBeDefined();
    expect(screen.getByText('updated')).toBeDefined();
    expect(entityHistory).toHaveBeenLastCalledWith(expect.anything(), 'tok-1', expect.anything());
    expect(screen.queryByRole('button', { name: 'Load older' })).toBeNull();
  });

  it('says when the entity has no recorded history, and shows a failed load instead of an empty list', async () => {
    entityHistory.mockResolvedValueOnce({ revisions: [], nextPageToken: '' });
    renderInspector();
    fireEvent.click(screen.getByRole('tab', { name: 'History' }));
    expect(await screen.findByText(/No recorded edits/)).toBeDefined();
    cleanup();

    entityHistory.mockRejectedValueOnce(new Error('deadline exceeded'));
    renderInspector();
    fireEvent.click(screen.getByRole('tab', { name: 'History' }));
    expect(await screen.findByText(/deadline exceeded/)).toBeDefined();
  });

  it('refetches from the newest page when the branch changes while the tab is open', async () => {
    entityHistory
      .mockResolvedValueOnce({ revisions: [revision('0192f000-0000-7000-8000-00000000beef')], nextPageToken: '' })
      .mockResolvedValueOnce({
        revisions: [revision('0192f000-0000-7000-8000-00000000cafe', { before: null, branch: 'alex/rework' })],
        nextPageToken: '',
      });
    renderInspector();
    fireEvent.click(screen.getByRole('tab', { name: 'History' }));
    expect(await screen.findByText('updated')).toBeDefined();
    expect(entityHistory).toHaveBeenLastCalledWith(
      expect.anything(),
      undefined,
      expect.objectContaining({ branch: undefined }),
    );

    fireEvent.click(screen.getByRole('button', { name: 'switch branch' }));
    expect(await screen.findByText('created')).toBeDefined();
    expect(screen.queryByText('updated')).toBeNull();
    expect(entityHistory).toHaveBeenLastCalledWith(
      expect.anything(),
      undefined,
      expect.objectContaining({ branch: 'alex/rework' }),
    );
  });
});

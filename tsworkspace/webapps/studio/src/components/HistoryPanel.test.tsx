import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/api')>();
  return { ...actual, api: { ...actual.api, changesets: vi.fn() } };
});

import { api, type ChangesetView } from '@/lib/api';
import { buildModel } from '@/lib/model';
import { HistoryPanel } from './HistoryPanel';

const changesets = vi.mocked(api.changesets);

const placed = { namespace: 'acme', slug: 'order.placed', version: '1' };
const model = buildModel([{ event: { id: placed, title: 'Order placed' } }] as never);

function changeset(id: string, overrides: Partial<ChangesetView> = {}): ChangesetView {
  return {
    id,
    author: 'acme-agent',
    at: '2026-01-02T00:00:00Z',
    message: 'batch mutate acme',
    branch: '',
    rpc: 'BatchMutate',
    ops: [
      { kind: 'KIND_PUT', entity: { kind: 'ENTITY_KIND_EVENT', ...placed } },
      {
        kind: 'KIND_DELETED',
        entity: { kind: 'ENTITY_KIND_COMMAND', namespace: 'acme', slug: 'order.cancel', version: '1' },
      },
    ],
    ...overrides,
  };
}

function renderPanel(
  props: { branch?: string; onSelect?: (key: string) => void; onViewAsOf?: (id: string) => void } = {},
) {
  const onSelect = props.onSelect ?? vi.fn();
  render(
    <HistoryPanel
      namespace="acme"
      branch={props.branch}
      model={model}
      onClose={vi.fn()}
      onSelect={onSelect}
      onViewAsOf={props.onViewAsOf}
    />,
  );
  return { onSelect };
}

beforeEach(() => {
  changesets.mockReset();
});

afterEach(cleanup);

describe('HistoryPanel', () => {
  it('lists the namespace changesets and expands one to its ops; an op in the model selects it', async () => {
    changesets.mockResolvedValue({
      changesets: [
        changeset('0192f000-0000-7000-8000-00000000beef'),
        changeset('0192f000-0000-7000-8000-00000000cafe', { rpc: 'MergeBranch', branch: 'alex/retention-rework' }),
      ],
      nextPageToken: '',
    });
    const { onSelect } = renderPanel();

    expect(screen.getByRole('heading', { name: 'Namespace history' })).toBeDefined();
    expect(await screen.findByText('BatchMutate')).toBeDefined();
    expect(screen.getByText('MergeBranch')).toBeDefined();
    expect(screen.getByText('alex/retention-rework')).toBeDefined();
    expect(screen.getAllByText('2 ops')).toHaveLength(2);
    expect(changesets).toHaveBeenCalledWith('acme', undefined, expect.objectContaining({ branch: undefined }));
    expect(screen.queryByRole('button', { name: 'Load older' })).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: /BatchMutate/ }));
    // A deleted command is not on the board, so it reads as text rather than a dead link.
    expect(screen.queryByRole('button', { name: /order\.cancel/ })).toBeNull();
    expect(screen.getByText(/acme\/order\.cancel/)).toBeDefined();

    fireEvent.click(screen.getByRole('button', { name: /acme\/order\.placed/ }));
    expect(onSelect).toHaveBeenCalledWith('event:acme/order.placed@1');
  });

  it('appends the next page when Load older is clicked', async () => {
    changesets
      .mockResolvedValueOnce({
        changesets: [changeset('0192f000-0000-7000-8000-00000000beef')],
        nextPageToken: 'tok-1',
      })
      .mockResolvedValueOnce({
        changesets: [changeset('0192f000-0000-7000-8000-00000000cafe', { rpc: 'PutEntity' })],
        nextPageToken: '',
      });
    renderPanel();

    fireEvent.click(await screen.findByRole('button', { name: 'Load older' }));
    expect(await screen.findByText('PutEntity')).toBeDefined();
    expect(screen.getByText('BatchMutate')).toBeDefined();
    expect(changesets).toHaveBeenLastCalledWith('acme', 'tok-1', expect.anything());
    expect(screen.queryByRole('button', { name: 'Load older' })).toBeNull();
  });

  it('reads the previewed branch', async () => {
    changesets.mockResolvedValue({ changesets: [], nextPageToken: '' });
    renderPanel({ branch: 'alex/retention-rework' });
    expect(await screen.findByText(/No recorded changesets/)).toBeDefined();
    expect(changesets).toHaveBeenCalledWith(
      'acme',
      undefined,
      expect.objectContaining({ branch: 'alex/retention-rework' }),
    );
  });

  it('offers the as-of view on a baseline changeset only', async () => {
    changesets.mockResolvedValue({
      changesets: [
        changeset('0192f000-0000-7000-8000-00000000beef'),
        changeset('0192f000-0000-7000-8000-00000000cafe', { rpc: 'MergeBranch', branch: 'alex/retention-rework' }),
      ],
      nextPageToken: '',
    });
    const onViewAsOf = vi.fn();
    renderPanel({ onViewAsOf });

    fireEvent.click(await screen.findByRole('button', { name: /MergeBranch/ }));
    expect(screen.queryByRole('button', { name: 'View board as of this' })).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: /BatchMutate/ }));
    fireEvent.click(screen.getByRole('button', { name: 'View board as of this' }));
    expect(onViewAsOf).toHaveBeenCalledWith('0192f000-0000-7000-8000-00000000beef');
  });

  it('offers no as-of view where the board cannot rewind', async () => {
    changesets.mockResolvedValue({
      changesets: [changeset('0192f000-0000-7000-8000-00000000beef')],
      nextPageToken: '',
    });
    renderPanel();
    fireEvent.click(await screen.findByRole('button', { name: /BatchMutate/ }));
    expect(screen.queryByRole('button', { name: 'View board as of this' })).toBeNull();
  });

  it('shows a failed load instead of an empty list', async () => {
    changesets.mockRejectedValueOnce(new Error('deadline exceeded'));
    renderPanel();
    expect(await screen.findByText(/deadline exceeded/)).toBeDefined();
    expect(screen.queryByText(/No recorded changesets/)).toBeNull();
  });
});

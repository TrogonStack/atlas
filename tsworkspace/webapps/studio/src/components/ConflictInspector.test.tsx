import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { BranchDiffEntry } from '@/lib/api';
import { ConflictInspector } from './ConflictInspector';

function conflictEntry(): BranchDiffEntry {
  const id = { namespace: 'shop', slug: 'order.placed', version: '1' };
  return {
    ref: { kind: 'ENTITY_KIND_EVENT', id },
    status: 'STATUS_CONFLICT_EDIT_EDIT',
    base: { event: { id, title: 'Original' } },
    ours: { event: { id, title: 'Ours' } },
    theirs: { event: { id, title: 'Theirs' } },
    conflictFieldPaths: ['event.title'],
  } as unknown as BranchDiffEntry;
}

function addedEntry(): BranchDiffEntry {
  const id = { namespace: 'shop', slug: 'order.shipped', version: '1' };
  return {
    ref: { kind: 'ENTITY_KIND_EVENT', id },
    status: 'STATUS_ADDED',
    ours: { event: { id, title: 'New event' } },
  } as unknown as BranchDiffEntry;
}

describe('ConflictInspector', () => {
  afterEach(cleanup);

  it('groups entries by status and names the branch', () => {
    render(<ConflictInspector branch="alex/rework" entries={[conflictEntry(), addedEntry()]} onClose={vi.fn()} />);
    expect(screen.getAllByText(/alex\/rework/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/conflict/i).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/added/i).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/order\.placed/).length).toBeGreaterThan(0);
  });

  it('shows the three panes and field paths for a selected conflict', async () => {
    render(<ConflictInspector branch="b" entries={[conflictEntry()]} onClose={vi.fn()} />);
    fireEvent.click(screen.getAllByText(/order\.placed/)[0]);
    expect(screen.getAllByText(/base/i).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/ours/i).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/theirs/i).length).toBeGreaterThan(0);
    expect(screen.getAllByText('event.title').length).toBeGreaterThan(0);
    expect(screen.getAllByText(/"Ours"/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/"Theirs"/).length).toBeGreaterThan(0);
  });

  it('closes via the close button and never offers a write action', async () => {
    const onClose = vi.fn();
    render(<ConflictInspector branch="b" entries={[conflictEntry()]} onClose={onClose} />);
    expect(screen.queryByRole('button', { name: /resolve|merge|apply/i })).toBeNull();
    const buttons = screen.getAllByRole('button');
    fireEvent.click(buttons[0]);
    expect(onClose).toHaveBeenCalled();
  });

  // Wire JSON may omit conflictFieldPaths on non-conflict statuses (and
  // occasionally on conflict rows). Selecting such an entry must not throw.
  it('selecting an added entry without conflictFieldPaths does not crash', () => {
    render(<ConflictInspector branch="b" entries={[addedEntry()]} onClose={vi.fn()} />);
    expect(() => fireEvent.click(screen.getAllByText(/order\.shipped/)[0])).not.toThrow();
    expect(screen.getAllByText(/ours/i).length).toBeGreaterThan(0);
  });
});

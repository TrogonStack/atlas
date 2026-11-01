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

  it.each([
    ['channel', 'ENTITY_KIND_CHANNEL'],
    ['component', 'ENTITY_KIND_COMPONENT'],
    ['externalSystem', 'ENTITY_KIND_EXTERNAL_SYSTEM'],
    ['dataProduct', 'ENTITY_KIND_DATA_PRODUCT'],
  ])('shows changed %s associations with exact selectors and distinct occurrence identities', (kind, wireKind) => {
    const id = { namespace: 'shop', slug: `${kind}-owner`, version: '18446744073709551615' };
    const target = { id: { ...id, slug: kind === 'channel' ? 'target-channel' : 'target-store' } };
    const beforeAssociation =
      kind === 'channel'
        ? {
            connectionId: { value: 'link-before' },
            kind: 'CHANNEL_LINK_KIND_COMPOSES',
            target,
            parameterValues: { tenant: '' },
            doc: { value: 'Before relationship' },
          }
        : {
            connectionId: { value: 'claim-before' },
            direction: 'STORAGE_DIRECTION_READ',
            store: target,
            ...(kind === 'dataProduct' ? { portId: { value: 'input-port' } } : {}),
            attachment: {
              owner: { kind: 'ENTITY_KIND_DATA_STORE', id: target.id },
              ownerUid: 'store-uid',
              id: { value: 'attachment' },
            },
          };
    const afterAssociation =
      kind === 'channel'
        ? {
            ...beforeAssociation,
            connectionId: { value: 'link-after' },
            kind: 'CHANNEL_LINK_KIND_ROUTES_TO',
            parameterValues: { tenant: 'north', region: 'eu' },
            doc: { value: '<script>inert()</script>' },
          }
        : {
            ...beforeAssociation,
            connectionId: { value: 'claim-after' },
            direction: 'STORAGE_DIRECTION_WRITE',
            ...(kind === 'dataProduct' ? { portId: { value: 'output-port' } } : {}),
          };
    const stable = { ...beforeAssociation, connectionId: { value: 'unchanged-sibling' } };
    const field = kind === 'channel' ? 'links' : 'storageClaims';
    const base = { [kind]: { id, title: 'Owner', [field]: [beforeAssociation, stable] }, system: { uid: 'owner-uid' } };
    const ours = { [kind]: { id, title: 'Owner', [field]: [afterAssociation, stable] }, system: { uid: 'owner-uid' } };
    const entry: BranchDiffEntry = {
      ref: { kind: wireKind, id },
      status: 'STATUS_CHANGED',
      base,
      ours,
      theirs: base,
      baseEtag: '11',
      conflictFieldPaths: [],
    };
    const { container } = render(<ConflictInspector branch="associations" entries={[entry]} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: new RegExp(`shop/${kind}-owner`) }));
    const panes = [...container.querySelectorAll('pre')].map((pane) => JSON.parse(pane.textContent ?? 'null'));
    expect(panes).toEqual([base, ours, base]);
    expect(panes[0][kind][field][0].connectionId.value).not.toBe(panes[1][kind][field][0].connectionId.value);
    expect(panes[0][kind][field][1]).toEqual(panes[1][kind][field][1]);
    expect(panes[1][kind].id.version).toBe('18446744073709551615');
    expect(container.querySelector('script')).toBeNull();
    expect(screen.queryByRole('button', { name: /resolve|merge|apply/i })).toBeNull();
  });
});

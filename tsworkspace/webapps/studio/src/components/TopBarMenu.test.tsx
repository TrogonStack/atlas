import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useState } from 'react';
import type { StudioFeatureFlags } from '@/lib/feature-flags';
import { TopBarMenu } from './TopBarMenu';

vi.mock('@/lib/webgl', () => ({ hasWebGL: () => false }));

function pendingExport() {
  let finish!: () => void;
  let fail!: (reason: Error) => void;
  const onExportHtml = vi.fn(
    () =>
      new Promise<void>((resolve, reject) => {
        finish = resolve;
        fail = reject;
      }),
  );
  return { onExportHtml, finish: () => finish(), fail: (reason: Error) => fail(reason) };
}

async function openMenu() {
  fireEvent.click(screen.getByRole('button', { name: 'More actions' }));
  await screen.findByRole('menuitem', { name: 'Refresh' });
}

describe('TopBarMenu', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  function PreviewMenu() {
    const [flags, setFlags] = useState<StudioFeatureFlags>({ laneMesh: false, streamTowers: false });
    return (
      <TopBarMenu
        onRefresh={vi.fn()}
        featureFlags={flags}
        onFeatureFlagChange={(feature, enabled) => setFlags((current) => ({ ...current, [feature]: enabled }))}
      />
    );
  }

  it('opens Feature Preview and preserves independent choices while browsing features', async () => {
    render(<PreviewMenu />);
    await openMenu();
    expect(screen.queryByRole('menuitemcheckbox')).toBeNull();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Feature Preview' }));

    const dialog = await screen.findByRole('dialog', { name: 'Feature Preview' });
    expect(screen.queryByRole('menu')).toBeNull();
    const previews = within(dialog);
    expect(previews.getAllByRole('tabpanel')).toHaveLength(1);
    expect(previews.getAllByRole('switch')).toHaveLength(1);
    const lanes = previews.getByRole('switch', { name: 'Enable Lane mesh' });
    expect(lanes.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(lanes);
    expect(lanes.getAttribute('aria-checked')).toBe('true');

    fireEvent.click(previews.getByRole('tab', { name: 'Stream towers' }));
    expect(previews.getAllByRole('tabpanel')).toHaveLength(1);
    expect(previews.getAllByRole('switch')).toHaveLength(1);
    const towers = previews.getByRole('switch', { name: 'Enable Stream towers' });
    expect(towers.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(towers);
    expect(towers.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(previews.getByRole('tab', { name: 'Lane mesh' }));
    expect(previews.getByRole('switch', { name: 'Enable Lane mesh' }).getAttribute('aria-checked')).toBe('true');

    fireEvent.click(previews.getByRole('button', { name: 'Close Feature Preview' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Feature Preview' }));
    const reopened = within(await screen.findByRole('dialog', { name: 'Feature Preview' }));
    expect(reopened.getByRole('switch', { name: 'Enable Lane mesh' }).getAttribute('aria-checked')).toBe('true');
    fireEvent.click(reopened.getByRole('tab', { name: 'Stream towers' }));
    expect(reopened.getByRole('switch', { name: 'Enable Stream towers' }).getAttribute('aria-checked')).toBe('true');
  });

  it('supports keyboard feature selection, closes with Escape, and restores focus to More actions', async () => {
    render(<PreviewMenu />);
    const trigger = screen.getByRole('button', { name: 'More actions' });
    await openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Feature Preview' }));
    const dialog = await screen.findByRole('dialog', { name: 'Feature Preview' });
    const previews = within(dialog);
    const closeButton = previews.getByRole('button', { name: 'Close Feature Preview' });
    await waitFor(() => expect(document.activeElement).toBe(closeButton));

    const laneTab = previews.getByRole('tab', { name: 'Lane mesh' });
    laneTab.focus();
    fireEvent.keyDown(laneTab, { key: 'ArrowDown', code: 'ArrowDown' });
    await waitFor(() =>
      expect(previews.getByRole('tab', { name: 'Stream towers' }).getAttribute('aria-selected')).toBe('true'),
    );
    const towers = previews.getByRole('switch', { name: 'Enable Stream towers' });
    towers.focus();
    fireEvent.keyDown(towers, { key: ' ', code: 'Space' });
    fireEvent.keyUp(towers, { key: ' ', code: 'Space' });
    expect(towers.getAttribute('aria-checked')).toBe('true');

    fireEvent.keyDown(dialog, { key: 'Escape', code: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  it('returns focus to More actions when closed with its close button', async () => {
    render(<PreviewMenu />);
    const trigger = screen.getByRole('button', { name: 'More actions' });
    await openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Feature Preview' }));
    const dialog = await screen.findByRole('dialog', { name: 'Feature Preview' });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Close Feature Preview' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  it('keeps Refresh reachable while an export is still running, and only locks Export HTML', async () => {
    const onRefresh = vi.fn();
    const { onExportHtml, finish } = pendingExport();
    render(<TopBarMenu onRefresh={onRefresh} onExportHtml={onExportHtml} />);

    await openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Export HTML' }));
    expect(onExportHtml).toHaveBeenCalledTimes(1);

    const trigger = screen.getByRole('button', { name: 'More actions' });
    expect(trigger.getAttribute('title')).toBe('Saving the board as HTML');
    expect((trigger as HTMLButtonElement).disabled).toBe(false);

    await openMenu();
    const exportItem = screen.getByRole('menuitem', { name: 'Export HTML' });
    expect(exportItem.getAttribute('aria-disabled')).toBe('true');
    fireEvent.click(exportItem);
    expect(onExportHtml).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole('menuitem', { name: 'Refresh' }));
    expect(onRefresh).toHaveBeenCalledTimes(1);

    finish();
    expect(await screen.findByTitle('Saved')).toBeDefined();
    await openMenu();
    expect(screen.getByRole('menuitem', { name: 'Export HTML' }).getAttribute('aria-disabled')).toBeNull();
  });

  it('announces export progress and completion through a status region', async () => {
    const { onExportHtml, finish } = pendingExport();
    render(<TopBarMenu onRefresh={vi.fn()} onExportHtml={onExportHtml} />);
    const status = screen.getByRole('status');
    expect(status.textContent).toBe('');

    await openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Export HTML' }));
    expect(status.textContent).toBe('Saving the board as HTML');

    finish();
    await waitFor(() => expect(status.textContent).toBe('Saved'));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('raises an alert that names the failure and its reason', async () => {
    const { onExportHtml, fail } = pendingExport();
    render(<TopBarMenu onRefresh={vi.fn()} onExportHtml={onExportHtml} />);

    await openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Export HTML' }));
    fail(new Error('Snapshot bundle missing'));

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toBe('Export failed: Snapshot bundle missing');
    expect(screen.getByText('Snapshot bundle missing')).toBeDefined();
    expect(screen.getByRole('status').textContent).toBe('');
    expect(screen.getByRole('button', { name: 'More actions' }).getAttribute('title')).toBe('Export failed');
  });
});

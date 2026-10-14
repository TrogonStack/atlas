import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { TopBarMenu } from './TopBarMenu';

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

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { TopBar } from './TopBar';

describe('TopBar', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders without crashing', () => {
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} />);
    expect(screen.getByRole('banner')).toBeDefined();
  });

  it('shows "live" indicator when connected', () => {
    render(<TopBar realtime={{ state: 'live' }} onRefresh={vi.fn()} />);
    expect(screen.getAllByText('live').length).toBeGreaterThan(0);
  });

  it('shows "offline" indicator when disconnected', () => {
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} />);
    expect(screen.getAllByText('offline').length).toBeGreaterThan(0);
  });

  it('keeps Refresh behind the actions menu and calls onRefresh from there', async () => {
    const onRefresh = vi.fn();
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={onRefresh} />);
    expect(screen.queryByRole('button', { name: 'Refresh' })).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'More actions' }));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Refresh' }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it('offers Export HTML in the menu only when the page can export', async () => {
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'More actions' }));
    await screen.findByRole('menuitem', { name: 'Refresh' });
    expect(screen.queryByRole('menuitem', { name: 'Export HTML' })).toBeNull();
    cleanup();

    const onExportHtml = vi.fn().mockResolvedValue(undefined);
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} onExportHtml={onExportHtml} />);
    fireEvent.click(screen.getByRole('button', { name: 'More actions' }));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Export HTML' }));
    expect(onExportHtml).toHaveBeenCalledTimes(1);
    expect(await screen.findByTitle('Saved')).toBeDefined();
  });

  it('shows why an export failed next to the menu', async () => {
    const onExportHtml = vi.fn().mockRejectedValue(new Error('Snapshot bundle missing'));
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} onExportHtml={onExportHtml} />);
    fireEvent.click(screen.getByRole('button', { name: 'More actions' }));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Export HTML' }));
    expect(await screen.findByText('Snapshot bundle missing')).toBeDefined();
  });

  it('displays the scoped title when provided', () => {
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} scopedTitle="My Model" />);
    expect(screen.getByText('My Model')).toBeDefined();
  });

  it('renders a back link when backHref is provided', () => {
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} backHref="/overview" />);
    const link = screen.getByRole('link', { name: /overview/i });
    expect((link as HTMLAnchorElement).href).toContain('/overview');
  });

  it('leaves the namespace registry to the overview instead of linking it from a model', () => {
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} />);
    expect(screen.queryByRole('link', { name: /namespaces/i })).toBeNull();
  });

  it('displays the schema version badge when info includes schemaVersion', () => {
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} info={{ schemaVersion: 'v2' }} />);
    expect(screen.getByText('v2')).toBeDefined();
  });

  it('displays the error message when error is provided', () => {
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} error="Something went wrong" />);
    expect(screen.getByText('Something went wrong')).toBeDefined();
  });

  // BUG: BranchBar is shown whenever onSelectBranch is set, and Review always
  // called `onOpenBranchReview?.()`. With a branch active but no review
  // handler, Review was a dead button.
  it('does not render a dead Review action when onOpenBranchReview is omitted', () => {
    render(
      <TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} branch="alex/rework" onSelectBranch={vi.fn()} />,
    );
    expect(screen.queryByRole('button', { name: 'Review' })).toBeNull();
  });

  it('renders Review when onOpenBranchReview is provided', () => {
    render(
      <TopBar
        realtime={{ state: 'offline' }}
        onRefresh={vi.fn()}
        branch="alex/rework"
        onSelectBranch={vi.fn()}
        onOpenBranchReview={vi.fn()}
      />,
    );
    expect(screen.getByRole('button', { name: 'Review' })).toBeDefined();
  });

  describe('snapshot', () => {
    const snapshot = { capturedAt: '2026-10-13T17:10:00.000Z', branch: 'alex/x' };

    it('replaces the live controls with when and where the snapshot was taken', () => {
      render(
        <TopBar
          realtime={{ state: 'live' }}
          onRefresh={vi.fn()}
          onSelectBranch={vi.fn()}
          backHref="/"
          info={{ serverVersion: '0.1.0' }}
          snapshot={snapshot}
        />,
      );
      const badge = screen.getByRole('status', { name: 'Snapshot' });
      expect(badge.textContent).toContain('branch alex/x');
      expect(badge.textContent).toContain('server 0.1.0');
      expect(screen.queryByRole('status', { name: 'Live updates' })).toBeNull();
      expect(screen.queryByRole('button', { name: 'More actions' })).toBeNull();
      expect(screen.queryByRole('link')).toBeNull();
    });

    it('labels a snapshot without a branch as the baseline', () => {
      render(
        <TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} snapshot={{ capturedAt: snapshot.capturedAt }} />,
      );
      expect(screen.getByRole('status', { name: 'Snapshot' }).textContent).toContain('baseline');
    });
  });

  it('never offers the actions menu inside a snapshot, so Export HTML cannot export an export', () => {
    render(
      <TopBar
        realtime={{ state: 'offline' }}
        onRefresh={vi.fn()}
        onExportHtml={vi.fn(() => Promise.resolve())}
        snapshot={{ capturedAt: '2026-10-13T17:10:00.000Z' }}
      />,
    );
    expect(screen.queryByRole('button', { name: 'More actions' })).toBeNull();
  });
});

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

  it('calls onRefresh when the Refresh button is clicked', () => {
    const onRefresh = vi.fn();
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={onRefresh} />);
    const buttons = screen.getAllByRole('button');
    const refreshButton = buttons.find((b) => /refresh/i.test(b.textContent ?? ''));
    expect(refreshButton).toBeDefined();
    fireEvent.click(refreshButton!);
    expect(onRefresh).toHaveBeenCalledTimes(1);
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

  it('always offers the namespace registry, which has no other entry point', () => {
    render(<TopBar realtime={{ state: 'offline' }} onRefresh={vi.fn()} />);
    const link = screen.getByRole('link', { name: /namespaces/i });
    expect((link as HTMLAnchorElement).href).toContain('/namespaces');
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
    expect(screen.queryByRole('button', { name: /review/i })).toBeNull();
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
    expect(screen.getByRole('button', { name: /review/i })).toBeDefined();
  });
});

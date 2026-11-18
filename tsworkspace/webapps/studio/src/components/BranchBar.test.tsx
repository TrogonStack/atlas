import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const branchesMock = vi.hoisted(() => vi.fn());

vi.mock('@/lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/api')>();
  return {
    ...actual,
    api: { ...actual.api, branches: branchesMock },
  };
});

import { BranchBar } from './BranchBar';

describe('BranchBar', () => {
  afterEach(cleanup);

  beforeEach(() => {
    branchesMock.mockReset();
  });

  it('names the active branch on the switcher, with a read-only hint and a Review action', async () => {
    const onOpenReview = vi.fn();
    render(<BranchBar branch="alex/rework" onSelectBranch={vi.fn()} onOpenReview={onOpenReview} />);
    const switcher = screen.getByRole('button', { name: /^Branch: alex\/rework/ });
    expect(switcher.textContent).toContain('read-only preview');
    fireEvent.click(screen.getByRole('button', { name: 'Review' }));
    expect(onOpenReview).toHaveBeenCalled();
  });

  it('names baseline on the switcher when no branch is active, with no preview hint or Review', () => {
    render(<BranchBar onSelectBranch={vi.fn()} onOpenReview={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Branch: baseline' })).toBeDefined();
    expect(screen.queryByText(/read-only preview/i)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Review' })).toBeNull();
  });

  it('lists branches from the api and selects one', async () => {
    branchesMock.mockResolvedValue({
      branches: [{ name: 'alex/a', doc: '', createdAt: '', deltaCount: 2 }],
    });
    const onSelectBranch = vi.fn();
    render(<BranchBar onSelectBranch={onSelectBranch} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /^Branch:/ }));
    await waitFor(() => expect(screen.getByText('alex/a')).toBeDefined());
    fireEvent.click(screen.getByText('alex/a'));
    expect(onSelectBranch).toHaveBeenCalledWith('alex/a');
  });

  it('offers baseline to exit the branch', async () => {
    branchesMock.mockResolvedValue({ branches: [] });
    const onSelectBranch = vi.fn();
    render(<BranchBar branch="alex/a" onSelectBranch={onSelectBranch} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /^Branch:/ }));
    await waitFor(() => expect(screen.getByText(/baseline/i)).toBeDefined());
    fireEvent.click(screen.getByText(/baseline/i));
    expect(onSelectBranch).toHaveBeenCalledWith(undefined);
  });

  it('renders the fetch error message when listing branches fails', async () => {
    branchesMock.mockRejectedValue(new Error('network down'));
    render(<BranchBar onSelectBranch={vi.fn()} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /^Branch:/ }));
    await waitFor(() => expect(screen.getByText('network down')).toBeDefined());
  });

  it('renders a stringified error when the rejection is not an Error instance', async () => {
    branchesMock.mockRejectedValue('boom');
    render(<BranchBar onSelectBranch={vi.fn()} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /^Branch:/ }));
    await waitFor(() => expect(screen.getByText('boom')).toBeDefined());
  });

  it('closes the dropdown when clicking outside it', async () => {
    branchesMock.mockResolvedValue({ branches: [] });
    render(<BranchBar onSelectBranch={vi.fn()} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /^Branch:/ }));
    await waitFor(() => expect(screen.getByText(/no branches yet/i)).toBeDefined());

    fireEvent.mouseDown(document.body);
    expect(screen.queryByText(/no branches yet/i)).toBeNull();
  });

  // BUG: once branches resolves, the effect guards on `branches !== undefined`
  // forever, so reopening the picker never refetches. A branch created via
  // trogon-atlas/MCP after the first open stays invisible until a full page reload.
  it('refetches the branch list each time the picker is opened', async () => {
    branchesMock.mockResolvedValueOnce({ branches: [] }).mockResolvedValueOnce({
      branches: [{ name: 'alex/new', doc: '', createdAt: '', deltaCount: 0 }],
    });
    render(<BranchBar onSelectBranch={vi.fn()} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /^Branch:/ }));
    await waitFor(() => expect(screen.getByText(/no branches yet/i)).toBeDefined());
    fireEvent.mouseDown(document.body);

    fireEvent.click(screen.getByRole('button', { name: /^Branch:/ }));
    await waitFor(() => expect(screen.getByText('alex/new')).toBeDefined());
    expect(branchesMock).toHaveBeenCalledTimes(2);
  });
});

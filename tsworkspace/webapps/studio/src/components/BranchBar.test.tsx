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

  it('shows the branch chip with a read-only hint when a branch is active', async () => {
    const onOpenReview = vi.fn();
    render(<BranchBar branch="alex/rework" onSelectBranch={vi.fn()} onOpenReview={onOpenReview} />);
    expect(screen.getByText('alex/rework')).toBeDefined();
    expect(screen.getByText(/read-only preview/i)).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: /review/i }));
    expect(onOpenReview).toHaveBeenCalled();
  });

  it('renders no chip on baseline', () => {
    render(<BranchBar onSelectBranch={vi.fn()} onOpenReview={vi.fn()} />);
    expect(screen.queryByText(/read-only preview/i)).toBeNull();
  });

  it('lists branches from the api and selects one', async () => {
    branchesMock.mockResolvedValue({
      branches: [{ name: 'alex/a', doc: '', createdAt: '', deltaCount: 2 }],
    });
    const onSelectBranch = vi.fn();
    render(<BranchBar onSelectBranch={onSelectBranch} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /branch/i }));
    await waitFor(() => expect(screen.getByText('alex/a')).toBeDefined());
    fireEvent.click(screen.getByText('alex/a'));
    expect(onSelectBranch).toHaveBeenCalledWith('alex/a');
  });

  it('offers baseline to exit the branch', async () => {
    branchesMock.mockResolvedValue({ branches: [] });
    const onSelectBranch = vi.fn();
    render(<BranchBar branch="alex/a" onSelectBranch={onSelectBranch} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /switch|branch/i }));
    await waitFor(() => expect(screen.getByText(/baseline/i)).toBeDefined());
    fireEvent.click(screen.getByText(/baseline/i));
    expect(onSelectBranch).toHaveBeenCalledWith(undefined);
  });

  it('renders the fetch error message when listing branches fails', async () => {
    branchesMock.mockRejectedValue(new Error('network down'));
    render(<BranchBar onSelectBranch={vi.fn()} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /branch/i }));
    await waitFor(() => expect(screen.getByText('network down')).toBeDefined());
  });

  it('renders a stringified error when the rejection is not an Error instance', async () => {
    branchesMock.mockRejectedValue('boom');
    render(<BranchBar onSelectBranch={vi.fn()} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /branch/i }));
    await waitFor(() => expect(screen.getByText('boom')).toBeDefined());
  });

  it('closes the dropdown when clicking outside it', async () => {
    branchesMock.mockResolvedValue({ branches: [] });
    render(<BranchBar onSelectBranch={vi.fn()} onOpenReview={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /branch/i }));
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

    fireEvent.click(screen.getByRole('button', { name: /branch/i }));
    await waitFor(() => expect(screen.getByText(/no branches yet/i)).toBeDefined());
    fireEvent.mouseDown(document.body);

    fireEvent.click(screen.getByRole('button', { name: /branch/i }));
    await waitFor(() => expect(screen.getByText('alex/new')).toBeDefined());
    expect(branchesMock).toHaveBeenCalledTimes(2);
  });
});

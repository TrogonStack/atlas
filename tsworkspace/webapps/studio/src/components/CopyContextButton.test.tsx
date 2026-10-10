import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CopyContextButton } from './CopyContextButton';

describe('CopyContextButton', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('copies the provided text via navigator.clipboard and flips the label', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    render(<CopyContextButton getText={() => 'hello context'} />);

    fireEvent.click(screen.getByRole('button', { name: /copy context/i }));

    await waitFor(() => {
      expect(screen.getByText('Copied')).toBeDefined();
    });
    expect(writeText).toHaveBeenCalledWith('hello context');
  });

  it('falls back to execCommand when clipboard API is unavailable', async () => {
    vi.stubGlobal('navigator', {});
    document.execCommand = vi.fn().mockReturnValue(true);
    render(<CopyContextButton getText={() => 'fallback text'} />);

    fireEvent.click(screen.getByRole('button', { name: /copy context/i }));

    await waitFor(() => {
      expect(screen.getByText('Copied')).toBeDefined();
    });
    expect(document.execCommand).toHaveBeenCalledWith('copy');
  });

  it('shows "Copy failed" when both clipboard and execCommand fail', async () => {
    vi.stubGlobal('navigator', {});
    document.execCommand = vi.fn().mockReturnValue(false);
    render(<CopyContextButton getText={() => 'text'} />);

    fireEvent.click(screen.getByRole('button', { name: /copy context/i }));

    await waitFor(() => {
      expect(screen.getByText('Copy failed')).toBeDefined();
    });
  });
});

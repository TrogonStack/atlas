import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CopyContextButton } from './CopyContextButton';

describe('CopyContextButton', () => {
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
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

describe('CopyContextButton lifecycle', () => {
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it.each([
    [true, 'Copied'],
    [false, 'Copy failed'],
  ] as const)('cancels feedback expiry after unmount (copy succeeds: %s)', async (ok, label) => {
    vi.useFakeTimers();
    vi.stubGlobal('navigator', {});
    document.execCommand = vi.fn().mockReturnValue(ok);
    const view = render(<CopyContextButton getText={() => 'context'} />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /copy context/i }));
    });
    expect(screen.getByText(label)).toBeDefined();
    view.unmount();
    act(() => vi.advanceTimersByTime(0));
    expect(vi.getTimerCount()).toBe(0);
  });

  it('does not schedule feedback expiry when the clipboard completes after unmount', async () => {
    vi.useFakeTimers();
    let finish: (() => void) | undefined;
    const copied = new Promise<void>((resolve) => {
      finish = resolve;
    });
    vi.stubGlobal('navigator', { clipboard: { writeText: vi.fn().mockReturnValue(copied) } });
    const view = render(<CopyContextButton getText={() => 'context'} />);
    fireEvent.click(screen.getByRole('button', { name: /copy context/i }));
    view.unmount();
    await act(async () => {
      finish?.();
      await copied;
    });
    act(() => vi.advanceTimersByTime(0));
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each([
    [true, 'Copied', 1500],
    [false, 'Copy failed', 2500],
  ] as const)('restores the label after feedback expires (copy succeeds: %s)', async (ok, label, delay) => {
    vi.useFakeTimers();
    vi.stubGlobal('navigator', {});
    document.execCommand = vi.fn().mockReturnValue(ok);
    render(<CopyContextButton getText={() => 'context'} />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /copy context/i }));
    });
    expect(screen.getByText(label)).toBeDefined();
    act(() => vi.advanceTimersByTime(delay));
    expect(screen.getByRole('button', { name: /copy context/i })).toBeDefined();
    expect(vi.getTimerCount()).toBe(0);
  });
});

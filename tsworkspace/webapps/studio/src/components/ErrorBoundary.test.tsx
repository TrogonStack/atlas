import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ErrorBoundary } from './ErrorBoundary';

function Bomb({ shouldThrow }: { shouldThrow: boolean }) {
  if (shouldThrow) throw new Error('kaboom');
  return <div>safe content</div>;
}

describe('ErrorBoundary', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders children when nothing throws', () => {
    render(
      <ErrorBoundary>
        <Bomb shouldThrow={false} />
      </ErrorBoundary>,
    );
    expect(screen.getByText('safe content')).toBeDefined();
  });

  it('catches a render error and shows the default fallback', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    render(
      <ErrorBoundary>
        <Bomb shouldThrow={true} />
      </ErrorBoundary>,
    );
    expect(screen.getByRole('alert')).toBeDefined();
    expect(screen.getByText('kaboom')).toBeDefined();
    expect(screen.getByRole('button', { name: 'Try again' })).toBeDefined();
    consoleError.mockRestore();
  });

  it('uses a custom fallback when provided', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    render(
      <ErrorBoundary fallback={({ error }) => <div>custom: {error.message}</div>}>
        <Bomb shouldThrow={true} />
      </ErrorBoundary>,
    );
    expect(screen.getByText('custom: kaboom')).toBeDefined();
    consoleError.mockRestore();
  });

  // BUG: getDerivedStateFromError stores a non-Error throw as-is. A string
  // throw is caught, but the default fallback reads `error.message` (always
  // undefined on a string) and shows a blank <pre>; the operator gets no
  // signal about what failed. Normalize to Error so the message is visible.
  it('surfaces the message from a non-Error throw in the default fallback', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    function StringBomb(): React.ReactNode {
      throw 'string-kaboom';
    }
    render(
      <ErrorBoundary>
        <StringBomb />
      </ErrorBoundary>,
    );
    expect(screen.getByRole('alert')).toBeDefined();
    expect(screen.getByText('string-kaboom')).toBeDefined();
    consoleError.mockRestore();
  });

  it('recovers on Try again after the throwing child is replaced', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { rerender } = render(
      <ErrorBoundary>
        <Bomb shouldThrow={true} />
      </ErrorBoundary>,
    );
    expect(screen.getByRole('alert')).toBeDefined();
    rerender(
      <ErrorBoundary>
        <Bomb shouldThrow={false} />
      </ErrorBoundary>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(screen.getByText('safe content')).toBeDefined();
    consoleError.mockRestore();
  });
});

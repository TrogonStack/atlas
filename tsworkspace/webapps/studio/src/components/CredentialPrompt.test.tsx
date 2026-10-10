import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getCredential, resetCredentialCacheForTests } from '@/lib/credential';
import { CredentialPrompt } from './CredentialPrompt';

beforeEach(() => {
  window.sessionStorage.clear();
  resetCredentialCacheForTests();
});

afterEach(() => {
  cleanup();
  window.sessionStorage.clear();
  resetCredentialCacheForTests();
});

describe('CredentialPrompt', () => {
  it('stores the key that was entered and dismisses', () => {
    const onDismiss = vi.fn();
    render(<CredentialPrompt onDismiss={onDismiss} />);
    fireEvent.change(screen.getByLabelText('API key'), { target: { value: 'alice-key' } });
    fireEvent.click(screen.getByRole('button', { name: /use this key/i }));
    expect(getCredential()).toBe('alice-key');
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it('masks the key as it is typed, since it is a bearer credential', () => {
    render(<CredentialPrompt />);
    expect(screen.getByLabelText('API key').getAttribute('type')).toBe('password');
  });

  it('refuses to submit an empty key rather than storing nothing and dismissing', () => {
    const onDismiss = vi.fn();
    render(<CredentialPrompt onDismiss={onDismiss} />);
    const button = screen.getByRole('button', { name: /use this key/i });
    expect(button.hasAttribute('disabled')).toBe(true);
    fireEvent.click(button);
    expect(onDismiss).not.toHaveBeenCalled();
    expect(getCredential()).toBeNull();
  });

  it('shows the reason the last attempt failed', () => {
    render(<CredentialPrompt message="unauthenticated" />);
    expect(screen.getByRole('alert').textContent).toContain('unauthenticated');
  });
});

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const registryMock = vi.hoisted(() => vi.fn());

vi.mock('@/lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/api')>();
  return {
    ...actual,
    api: {
      ...actual.api,
      namespaceRegistry: registryMock,
    },
  };
});

import { NamespacesPage } from './NamespacesPage';

const row = (over: Partial<Record<string, unknown>> = {}) => ({
  id: 'ns_0192abcd',
  name: 'billing',
  parent: 'acme',
  entityCount: 4,
  registered: true,
  ...over,
});

describe('NamespacesPage', () => {
  afterEach(cleanup);

  beforeEach(() => {
    registryMock.mockReset();
    registryMock.mockResolvedValue({ namespaces: [] });
  });

  it('shows the id, because it is what an agent move needs and nothing else reports it', async () => {
    registryMock.mockResolvedValue({ namespaces: [row()] });
    render(<NamespacesPage />);
    await waitFor(() => expect(screen.getByText('ns_0192abcd')).toBeDefined());
    expect(screen.getByText('acme')).toBeDefined();
    expect(screen.getByText('4')).toBeDefined();
  });

  it('calls out a namespace that holds entities but has no owner', async () => {
    registryMock.mockResolvedValue({
      namespaces: [row({ id: '', parent: '', registered: false, name: 'orphan' })],
    });
    render(<NamespacesPage />);

    // The point of the banner: a scoped key does not see this namespace at
    // all, and no other screen in the studio can tell you that.
    await waitFor(() => expect(screen.getByText(/no registry row/i)).toBeDefined());
    expect(screen.getByText('unregistered')).toBeDefined();
  });

  it('says nothing about unowned namespaces when every row has an owner', async () => {
    registryMock.mockResolvedValue({ namespaces: [row()] });
    render(<NamespacesPage />);
    await waitFor(() => expect(screen.getByText('billing')).toBeDefined());
    expect(screen.queryByText(/no registry row/i)).toBeNull();
  });

  it('offers no control that registers, claims, or moves a namespace', async () => {
    registryMock.mockResolvedValue({
      namespaces: [row(), row({ id: '', parent: '', registered: false, name: 'orphan' })],
    });
    render(<NamespacesPage />);
    await waitFor(() => expect(screen.getByText('billing')).toBeDefined());

    expect(screen.queryByRole('button', { name: /register/i })).toBeNull();
    expect(screen.queryByRole('button', { name: /claim/i })).toBeNull();
    expect(screen.queryByRole('button', { name: /move/i })).toBeNull();
    expect(screen.queryByLabelText('namespace name')).toBeNull();
    expect(screen.queryByLabelText('owner')).toBeNull();
  });

  it('still refreshes the registry on request', async () => {
    registryMock.mockResolvedValue({ namespaces: [row()] });
    render(<NamespacesPage />);
    await waitFor(() => expect(registryMock).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));

    await waitFor(() => expect(registryMock).toHaveBeenCalledTimes(2));
  });

  it('filters on the id as well as the name, so a minted id is findable', async () => {
    registryMock.mockResolvedValue({
      namespaces: [row(), row({ id: 'ns_ffff', name: 'shipping', parent: 'beta' })],
    });
    render(<NamespacesPage />);
    await waitFor(() => expect(screen.getByText('shipping')).toBeDefined());

    fireEvent.change(screen.getByLabelText('filter namespaces'), { target: { value: 'ns_ffff' } });

    expect(screen.getByText('shipping')).toBeDefined();
    expect(screen.queryByText('billing')).toBeNull();
  });

  it('shows the load failure instead of an empty registry', async () => {
    registryMock.mockRejectedValue(new Error('upstream unavailable'));
    render(<NamespacesPage />);
    await waitFor(() => expect(screen.getByText(/upstream unavailable/)).toBeDefined());
  });
});

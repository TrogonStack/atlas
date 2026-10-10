import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { buildModel } from '@/lib/model';
import { Sidebar } from './Sidebar';

vi.mock('@/lib/api', () => ({
  api: {
    search: vi.fn(),
  },
}));

import { api } from '@/lib/api';

const emptyModel = buildModel([]);

function renderSidebar(overrides?: Partial<Parameters<typeof Sidebar>[0]>) {
  const onNamespaces = vi.fn();
  const onSelect = vi.fn();
  const result = render(
    <Sidebar
      model={emptyModel}
      namespaces={[]}
      allNamespaces={['shop', 'payments']}
      namespaceLabels={{}}
      branch={undefined}
      onNamespaces={onNamespaces}
      onSelect={onSelect}
      {...overrides}
    />,
  );
  return { ...result, onNamespaces, onSelect };
}

describe('Sidebar search', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    cleanup();
  });

  it('fires a search after the debounce delay', async () => {
    (api.search as ReturnType<typeof vi.fn>).mockResolvedValue({ results: [] });
    const { container } = renderSidebar();

    const input = within(container).getByPlaceholderText('Search the model…');
    fireEvent.change(input, { target: { value: 'order' } });

    expect(api.search).not.toHaveBeenCalled();

    await vi.runAllTimersAsync();

    expect(api.search).toHaveBeenCalledWith('order', [], expect.objectContaining({ signal: expect.any(AbortSignal) }));
  });

  it('searches the active branch rather than baseline', async () => {
    // A branch preview that finds nothing is indistinguishable from a
    // branch that lost the entity, which is what baseline search looked
    // like from inside a preview.
    (api.search as ReturnType<typeof vi.fn>).mockResolvedValue({ results: [] });
    const { container } = renderSidebar({ branch: 'alex/retention-rework' });

    fireEvent.change(within(container).getByPlaceholderText('Search the model…'), {
      target: { value: 'order' },
    });
    await vi.runAllTimersAsync();

    expect(api.search).toHaveBeenCalledWith('order', [], expect.objectContaining({ branch: 'alex/retention-rework' }));
  });

  it('does not update hits when the request is aborted by a subsequent keystroke', async () => {
    let resolveFirst!: (v: { results: unknown[] }) => void;
    const firstSearch = new Promise<{ results: unknown[] }>((r) => {
      resolveFirst = r;
    });

    (api.search as ReturnType<typeof vi.fn>).mockReturnValueOnce(firstSearch).mockResolvedValue({ results: [] });

    const { container } = renderSidebar();

    const input = within(container).getByPlaceholderText('Search the model…');

    fireEvent.change(input, { target: { value: 'ord' } });
    await vi.runAllTimersAsync();

    fireEvent.change(input, { target: { value: 'order' } });
    await vi.runAllTimersAsync();

    expect(api.search).toHaveBeenCalledTimes(2);

    resolveFirst({ results: [] });
    await Promise.resolve();

    expect(within(container).queryByRole('alert')).toBeNull();
  });

  it('clears results when the query is emptied', async () => {
    (api.search as ReturnType<typeof vi.fn>).mockResolvedValue({ results: [] });
    const { container } = renderSidebar();

    const input = within(container).getByPlaceholderText('Search the model…');
    fireEvent.change(input, { target: { value: 'order' } });
    fireEvent.change(input, { target: { value: '' } });

    await vi.runAllTimersAsync();

    expect(api.search).not.toHaveBeenCalled();
    expect(within(container).queryByText(/Results/)).toBeNull();
  });
});

describe('Sidebar namespace picker', () => {
  afterEach(cleanup);

  it('shows the human name while still filtering by the id', () => {
    // The registry mints opaque ids; only the name means anything to a
    // person, and only the id means anything to the server.
    const { getByRole, onNamespaces } = renderSidebar({
      allNamespaces: ['ns_0199aa', 'ns_0199bb'],
      namespaceLabels: { ns_0199aa: 'orders', ns_0199bb: 'payments' },
    });

    getByRole('checkbox', { name: 'orders' });
    fireEvent.click(getByRole('checkbox', { name: 'orders' }));

    // Deselecting from "all" means "everything except this one", and what
    // travels back is the id of the one left over, never its label.
    expect(onNamespaces).toHaveBeenCalledWith(['ns_0199bb']);
  });

  it('falls back to the id when no label was supplied', () => {
    // Namespaces adopted before the registry are their own id, and a
    // scoped session never fetches the label map at all.
    const { getByRole } = renderSidebar({
      allNamespaces: ['legacy-ns'],
      namespaceLabels: {},
    });

    getByRole('checkbox', { name: 'legacy-ns' });
  });
});

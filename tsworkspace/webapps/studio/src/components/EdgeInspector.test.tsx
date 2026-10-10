import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: () => <button type="button">Copy context</button>,
}));

vi.mock('@/components/canvas/StickyNode', () => ({
  kindStyle: (kind: string) => ({ label: kind }),
}));

vi.mock('@/lib/context', () => ({
  edgeContext: vi.fn(() => 'edge context string'),
}));

import type { BoardEdgeData } from '@/lib/layout';
import type { Entity } from '@/lib/model';
import { EdgeInspector } from './EdgeInspector';

function makeEntity(key: string, title: string, kind: Entity['kind'] = 'event'): Entity {
  return {
    kind,
    id: { namespace: 'test', slug: key, version: '1' },
    key,
    title,
    doc: '',
    annotations: [],
    fields: [],
    raw: {},
  };
}

function makeEdgeData(overrides: Partial<BoardEdgeData> = {}): BoardEdgeData {
  return {
    relation: 'triggers',
    doc: '',
    metadata: [],
    source: makeEntity('cmd', 'Submit Order', 'command'),
    target: makeEntity('evt', 'Order Placed', 'event'),
    ...overrides,
  };
}

describe('EdgeInspector', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders without crashing', () => {
    render(<EdgeInspector edge={makeEdgeData()} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Connection')).toBeDefined();
  });

  it('displays the source entity title', () => {
    render(<EdgeInspector edge={makeEdgeData()} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getAllByText('Submit Order').length).toBeGreaterThan(0);
  });

  it('displays the target entity title', () => {
    render(<EdgeInspector edge={makeEdgeData()} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getAllByText('Order Placed').length).toBeGreaterThan(0);
  });

  it('displays the relation name in the heading', () => {
    render(<EdgeInspector edge={makeEdgeData({ relation: 'emits' })} onClose={vi.fn()} onSelect={vi.fn()} />);
    const headings = screen.getAllByRole('heading', { level: 2 });
    expect(headings.length).toBeGreaterThan(0);
    expect(headings[0].textContent).toBe('emits');
  });

  it('calls onClose when the close button is clicked', () => {
    const onClose = vi.fn();
    render(<EdgeInspector edge={makeEdgeData()} onClose={onClose} onSelect={vi.fn()} />);
    const buttons = screen.getAllByRole('button');
    const closeButton = buttons.find((b) => !b.textContent?.includes('Copy') && b.querySelector('svg'));
    expect(closeButton).toBeDefined();
    fireEvent.click(closeButton!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onSelect when a source endpoint button is clicked', () => {
    const onSelect = vi.fn();
    render(<EdgeInspector edge={makeEdgeData()} onClose={vi.fn()} onSelect={onSelect} />);
    const submitBtn = screen.getAllByText('Submit Order')[0].closest('button');
    expect(submitBtn).toBeDefined();
    fireEvent.click(submitBtn!);
    expect(onSelect).toHaveBeenCalledWith('cmd');
  });

  it('shows the doc section when doc is provided', () => {
    render(
      <EdgeInspector
        edge={makeEdgeData({ doc: 'This is some documentation.' })}
        onClose={vi.fn()}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.getByText('This is some documentation.')).toBeDefined();
  });

  it('shows the empty state message when no doc or metadata', () => {
    render(<EdgeInspector edge={makeEdgeData({ doc: '', metadata: [] })} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getAllByText(/No design data is attached/).length).toBeGreaterThan(0);
  });
});

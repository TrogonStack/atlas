import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: ({ getText }: { getText: () => string }) => (
    <button type="button" onClick={getText}>
      Copy context
    </button>
  ),
}));

vi.mock('@/components/canvas/StickyNode', () => ({
  kindStyle: (kind: string) => ({ label: kind }),
}));

vi.mock('@/lib/context', () => ({
  entityContext: vi.fn(() => 'entity context'),
  edgeContext: vi.fn(() => 'edge context'),
}));

import type { SelectedEdge } from '@/components/canvas/Board';
import type { Entity } from '@/lib/model';
import { buildModel } from '@/lib/model';
import { MultiInspector } from './MultiInspector';

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

function makeEdge(id: string, sourceTitle: string, targetTitle: string): SelectedEdge {
  return {
    id,
    data: {
      relation: 'triggers',
      doc: '',
      metadata: [],
      source: makeEntity(`src-${id}`, sourceTitle),
      target: makeEntity(`tgt-${id}`, targetTitle),
    },
  };
}

describe('MultiInspector', () => {
  const model = buildModel([]);

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders without crashing with empty selections', () => {
    render(
      <MultiInspector
        model={model}
        entities={[]}
        edges={[]}
        onClose={vi.fn()}
        onSelect={vi.fn()}
        onSelectEdge={vi.fn()}
      />,
    );
    expect(screen.getByText('0 selected')).toBeDefined();
  });

  it('shows the correct total count of entities and edges', () => {
    const entities = [makeEntity('e1', 'Order Placed'), makeEntity('e2', 'Order Shipped')];
    const edges = [makeEdge('edge-1', 'Order Placed', 'Notify Service')];
    render(
      <MultiInspector
        model={model}
        entities={entities}
        edges={edges}
        onClose={vi.fn()}
        onSelect={vi.fn()}
        onSelectEdge={vi.fn()}
      />,
    );
    expect(screen.getByText('3 selected')).toBeDefined();
  });

  it('renders entity titles in the list', () => {
    const entities = [makeEntity('e1', 'Order Placed')];
    render(
      <MultiInspector
        model={model}
        entities={entities}
        edges={[]}
        onClose={vi.fn()}
        onSelect={vi.fn()}
        onSelectEdge={vi.fn()}
      />,
    );
    expect(screen.getAllByText('Order Placed').length).toBeGreaterThan(0);
  });

  it('calls onClose when the close button is clicked', () => {
    const onClose = vi.fn();
    render(
      <MultiInspector
        model={model}
        entities={[]}
        edges={[]}
        onClose={onClose}
        onSelect={vi.fn()}
        onSelectEdge={vi.fn()}
      />,
    );
    const buttons = screen.getAllByRole('button');
    const closeButton = buttons.find((b) => !b.textContent?.includes('Copy') && b.querySelector('svg'));
    expect(closeButton).toBeDefined();
    fireEvent.click(closeButton!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onSelect when an entity row is clicked', () => {
    const onSelect = vi.fn();
    const entities = [makeEntity('e1', 'Order Placed')];
    render(
      <MultiInspector
        model={model}
        entities={entities}
        edges={[]}
        onClose={vi.fn()}
        onSelect={onSelect}
        onSelectEdge={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByTitle('Click to focus this entity'));
    expect(onSelect).toHaveBeenCalledWith('e1');
  });
});

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: () => <button type="button">Copy context</button>,
}));

vi.mock('@/lib/context', () => ({
  entityContext: vi.fn(() => 'entity context'),
  edgeContext: vi.fn(() => 'edge context'),
}));

import type { Entity } from '@/lib/model';
import { buildModel } from '@/lib/model';
import { MultiInspector } from './MultiInspector';

describe('MultiInspector kind badges', () => {
  afterEach(cleanup);

  it('renders human labels for slice and structural kinds (not raw camelCase)', () => {
    const model = buildModel([
      {
        commandSlice: {
          id: { namespace: 'shop', slug: 'place', version: '1' },
          title: 'Place order slice',
        },
      },
      {
        storyboard: {
          id: { namespace: 'shop', slug: 'checkout', version: '1' },
          title: 'Checkout board',
        },
      },
    ] as never);
    const entities = model.entities.filter((e) => e.kind === 'commandSlice' || e.kind === 'storyboard') as Entity[];
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
    expect(screen.getByText('Command Slice')).toBeDefined();
    expect(screen.getByText('Storyboard')).toBeDefined();
    expect(screen.queryByText('commandSlice')).toBeNull();
    expect(screen.queryByText('storyboard')).toBeNull();
  });
});

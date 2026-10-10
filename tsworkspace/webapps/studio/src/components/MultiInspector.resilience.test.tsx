import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: () => <button type="button">Copy context</button>,
}));

vi.mock('@/lib/context', () => ({
  entityContext: vi.fn(() => 'entity context'),
  edgeContext: vi.fn(() => 'edge context'),
}));

import type { SelectedEdge } from '@/components/canvas/Board';
import { buildModel } from '@/lib/model';
import { MultiInspector } from './MultiInspector';

describe('MultiInspector resilience', () => {
  afterEach(cleanup);

  it('survives partial edge payloads missing source/target titles', () => {
    const edge = {
      id: 'seam:x',
      data: { relation: 'feeds context', doc: '', metadata: [] },
    } as unknown as SelectedEdge;
    expect(() =>
      render(
        <MultiInspector
          model={buildModel([])}
          entities={[]}
          edges={[edge]}
          onClose={vi.fn()}
          onSelect={vi.fn()}
          onSelectEdge={vi.fn()}
        />,
      ),
    ).not.toThrow();
    expect(screen.getByText('1 selected')).toBeDefined();
    expect(screen.getByTitle('Click to focus this connection').textContent).toMatch(/\?/);
  });
});

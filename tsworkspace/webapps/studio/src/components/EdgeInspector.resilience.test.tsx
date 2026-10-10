import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: () => <button type="button">Copy context</button>,
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

describe('EdgeInspector resilience', () => {
  afterEach(cleanup);

  it('survives missing metadata (runtime edge data holes)', () => {
    const edge = {
      relation: 'triggers',
      doc: '',
      source: makeEntity('cmd', 'Submit'),
      target: makeEntity('evt', 'Placed'),
    } as BoardEdgeData;
    expect(() => render(<EdgeInspector edge={edge} onClose={vi.fn()} onSelect={vi.fn()} />)).not.toThrow();
    expect(screen.getByText('Connection')).toBeDefined();
  });

  it('falls back the mid-arrow relation label when relation is empty', () => {
    const edge: BoardEdgeData = {
      relation: '',
      doc: '',
      metadata: [],
      source: makeEntity('cmd', 'Submit', 'command'),
      target: makeEntity('evt', 'Placed'),
    };
    const { container } = render(<EdgeInspector edge={edge} onClose={vi.fn()} onSelect={vi.fn()} />);
    const mid = container.querySelector('span.ml-1.text-\\[10px\\].uppercase') as HTMLElement | null;
    // Without a fallback this mid-arrow label is blank while the heading says "connection".
    expect(mid?.textContent?.trim().toLowerCase()).toBe('connection');
  });
});

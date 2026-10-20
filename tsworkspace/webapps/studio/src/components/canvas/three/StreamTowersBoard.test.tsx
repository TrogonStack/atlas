import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SceneSpec } from '@/lib/scene3d';

const seen = vi.hoisted(() => ({ specs: [] as SceneSpec[] }));

vi.mock('./GraphBoard', () => ({
  GraphBoard: ({ spec }: { spec: SceneSpec }) => {
    seen.specs.push(spec);
    return <div data-testid="graph-board" />;
  },
}));

import { buildModel } from '@/lib/model';
import { storefrontEntities } from '@/lib/scene3d.fixture';
import StreamTowersBoard from './StreamTowersBoard';

const base = buildModel(storefrontEntities() as never);

describe('StreamTowersBoard', () => {
  afterEach(() => {
    seen.specs = [];
    cleanup();
  });

  it('reuses the towers spec when the model changes without layout changes', () => {
    const { rerender } = render(<StreamTowersBoard model={base} selections={[]} onSelect={() => {}} />);
    const first = seen.specs.at(-1);
    rerender(
      <StreamTowersBoard model={{ ...base, entities: [...base.entities] }} selections={[]} onSelect={() => {}} />,
    );
    expect(seen.specs.at(-1)).toBe(first);
  });
});

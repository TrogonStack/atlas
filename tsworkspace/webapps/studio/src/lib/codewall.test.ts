import { describe, expect, it } from 'vitest';
import { bundleOf, codeRefsOf, layoutCodeWall, type WallEntry } from './codewall';
import { buildModel, type Entity } from './model';

const CODE_REF = 'type.googleapis.com/trogonatlas.annotation.v1alpha1.CodeRefAnnotation';

function entityWith(metadata: Record<string, unknown>[]): Entity {
  const model = buildModel([
    { command: { id: { namespace: 'shop', slug: 'place-order', version: '1' }, title: 'Place order', metadata } },
  ] as never);
  return model.entities[0];
}

describe('codeRefsOf', () => {
  it('reads the repo, path, symbol and ref of each code reference', () => {
    const e = entityWith([
      { '@type': CODE_REF, repo: 'shop', path: '/lib/orders/place.ex', symbol: 'Orders.Place', ref: 'v2' },
    ]);
    expect(codeRefsOf(e)).toEqual([{ repo: 'shop', path: 'lib/orders/place.ex', symbol: 'Orders.Place', ref: 'v2' }]);
  });

  it('skips a reference without a repo or a path, and other annotations', () => {
    const e = entityWith([
      { '@type': CODE_REF, repo: 'shop' },
      { '@type': CODE_REF, path: 'lib/a.ex' },
      { '@type': CODE_REF, repo: ' ', path: 'lib/a.ex' },
      { '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.DocAnnotation', text: 'hello' },
    ]);
    expect(codeRefsOf(e)).toEqual([]);
  });
});

describe('bundleOf', () => {
  it('groups by repo and at most the first two directories', () => {
    expect(bundleOf({ repo: 'shop', path: 'apps/web/lib/orders/place.ex' })).toEqual({
      id: JSON.stringify(['shop', 'apps/web']),
      repo: 'shop',
      dir: 'apps/web',
    });
    expect(bundleOf({ repo: 'shop', path: 'lib/place.ex' }).dir).toBe('lib');
    expect(bundleOf({ repo: 'shop', path: 'main.go' })).toEqual({
      id: JSON.stringify(['shop', '']),
      repo: 'shop',
      dir: '',
    });
  });

  it('keeps the same directory in two repos apart', () => {
    expect(bundleOf({ repo: 'a', path: 'lib/x.ex' }).id).not.toBe(bundleOf({ repo: 'b', path: 'lib/x.ex' }).id);
  });

  it('keeps a colon in a repo name from merging two bundles', () => {
    expect(bundleOf({ repo: 'a:b', path: 'c/x.ex' }).id).not.toBe(bundleOf({ repo: 'a', path: 'b:c/x.ex' }).id);
  });
});

const entry = (key: string, x: number, ...paths: string[]): WallEntry => ({
  key,
  x,
  refs: paths.map((p) => {
    const [repo, path] = p.split(':');
    return { repo, path, symbol: `${key}.symbol` };
  }),
});

describe('layoutCodeWall', () => {
  it('has no wall when nothing cites a file', () => {
    expect(layoutCodeWall([], { top: 0, z: -10 })).toBeUndefined();
    expect(layoutCodeWall([{ key: 'a', x: 0, refs: [] }], { top: 0, z: -10 })).toBeUndefined();
  });

  it('merges the entities that cite one file onto that file', () => {
    const wall = layoutCodeWall([entry('a', 0, 'shop:lib/x.ex'), entry('b', 100, 'shop:lib/x.ex')], { top: 0, z: -10 });
    expect(wall?.files).toHaveLength(1);
    expect(wall?.files[0].citedBy).toEqual(['a', 'b']);
    expect(wall?.files[0].symbols).toEqual(['a.symbol', 'b.symbol']);
    expect(wall?.files[0].name).toBe('x.ex');
  });

  it('hangs a single bundle under the cards that cite it', () => {
    const wall = layoutCodeWall([entry('a', 300, 'shop:lib/x.ex', 'shop:lib/y.ex')], { top: -50, z: -10 });
    expect(wall?.bundles).toHaveLength(1);
    const bundle = wall?.bundles[0];
    expect(bundle?.center.x).toBeCloseTo(300);
    expect((bundle?.center.y ?? 0) + (bundle?.height ?? 0) / 2).toBeCloseTo(-50);
    expect(bundle?.center.z).toBe(-10);
  });

  it('puts bundles side by side in the order of their cards, with no overlap', () => {
    const wall = layoutCodeWall(
      [
        entry('c', 40, 'pay:services/capture/worker.go'),
        entry('a', 0, 'shop:apps/web/a.ex', 'shop:apps/web/b.ex', 'shop:apps/web/c.ex'),
        entry('b', 20, 'shop:apps/core/x.ex'),
      ],
      { top: 0, z: -10 },
    );
    const bundles = wall?.bundles ?? [];
    expect(bundles.map((b) => b.id)).toEqual([
      JSON.stringify(['shop', 'apps/web']),
      JSON.stringify(['shop', 'apps/core']),
      JSON.stringify(['pay', 'services/capture']),
    ]);
    for (let i = 1; i < bundles.length; i++) {
      const left = bundles[i - 1];
      const right = bundles[i];
      expect(left.center.x + left.width / 2).toBeLessThan(right.center.x - right.width / 2);
    }
    for (const b of bundles) expect(b.center.y + b.height / 2).toBeCloseTo(0);
  });

  it('places each file inside its bundle, in the order of the cards that cite it', () => {
    const wall = layoutCodeWall(
      [entry('late', 200, 'shop:lib/z.ex'), entry('early', 0, 'shop:lib/a.ex'), entry('mid', 100, 'shop:lib/m.ex')],
      { top: 0, z: -10 },
    );
    const bundle = wall?.bundles[0];
    expect(bundle?.files.map((id) => wall?.files.find((f) => f.id === id)?.name)).toEqual(['a.ex', 'm.ex', 'z.ex']);
    for (const f of wall?.files ?? []) {
      expect(Math.abs(f.center.x - (bundle?.center.x ?? 0))).toBeLessThan((bundle?.width ?? 0) / 2);
      expect(Math.abs(f.center.y - (bundle?.center.y ?? 0))).toBeLessThan((bundle?.height ?? 0) / 2);
    }
  });

  it('lays out the same entries the same way every time', () => {
    const entries = [entry('a', 0, 'shop:lib/a.ex'), entry('b', 0, 'shop:src/b.ex'), entry('c', 0, 'pay:src/c.go')];
    expect(layoutCodeWall(entries, { top: 0, z: -10 })).toEqual(layoutCodeWall(entries, { top: 0, z: -10 }));
  });
});

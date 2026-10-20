import { Mesh, PlaneGeometry, TubeGeometry } from 'three';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SceneSpec } from '@/lib/scene3d';

vi.mock('three/addons/controls/OrbitControls.js', () => ({
  OrbitControls: class {
    target = { x: 0, y: 0, z: 0, copy: () => {} };
    addEventListener() {}
    removeEventListener() {}
    dispose() {}
  },
}));

vi.mock('three/addons/renderers/CSS2DRenderer.js', () => ({
  CSS2DObject: class {
    position = { copy: () => {} };
  },
  CSS2DRenderer: class {
    domElement = document.createElement('div');
    setSize() {}
    render() {}
  },
}));

vi.mock('three', async (importOriginal) => {
  const actual = await importOriginal<typeof import('three')>();
  return {
    ...actual,
    WebGLRenderer: class {
      domElement = document.createElement('canvas');
      capabilities = { getMaxAnisotropy: () => 1 };
      setPixelRatio() {}
      setSize() {}
      setClearColor() {}
      dispose() {}
      forceContextLoss() {}
      render() {}
    },
  };
});

import { GraphScene } from './scene';

function emptySpec(plates: SceneSpec['plates']): SceneSpec {
  const region = { min: { x: 0, y: 0, z: 0 }, max: { x: 100, y: 100, z: 100 } };
  return {
    nodes: [],
    files: [],
    links: [],
    plates,
    tags: [],
    decor: [],
    front: region,
    frontDirection: { x: 0, y: 0, z: 1 },
    overview: { region, direction: { x: 0, y: 0, z: 1 } },
    steps: [],
    stepDirection: { x: 0, y: 1, z: 0 },
    stepNoun: 'Slice',
    legend: [],
  };
}

describe('GraphScene plates', () => {
  let host: HTMLDivElement;
  let disposeSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        disconnect() {}
      },
    );
    host = document.createElement('div');
    document.body.appendChild(host);
    Object.defineProperty(host, 'clientWidth', { value: 800, configurable: true });
    Object.defineProperty(host, 'clientHeight', { value: 600, configurable: true });
    disposeSpy = vi.spyOn(PlaneGeometry.prototype, 'dispose');
  });

  afterEach(() => {
    disposeSpy.mockRestore();
    host.remove();
  });

  it('disposes the temporary plane after building a wide header frame', () => {
    const scene = new GraphScene(host, { pick: () => {}, hover: () => {}, lost: () => {} });
    disposeSpy.mockClear();
    scene.setSpec(emptySpec([{ id: 'p1', center: { x: 0, y: 0, z: 0 }, width: 400, height: 20, title: 'Lane' }]));
    expect(disposeSpy).toHaveBeenCalled();
    scene.dispose();
  });

  it('does not dispose plate meshes while drawing a narrow header', () => {
    const scene = new GraphScene(host, { pick: () => {}, hover: () => {}, lost: () => {} });
    disposeSpy.mockClear();
    scene.setSpec(emptySpec([{ id: 'p1', center: { x: 0, y: 0, z: 0 }, width: 200, height: 20, title: 'Lane' }]));
    expect(disposeSpy).not.toHaveBeenCalled();
    scene.dispose();
  });

  it('registers link tubes as pick targets so a click hits the tube, not the card behind', () => {
    const scene = new GraphScene(host, { pick: () => {}, hover: () => {}, lost: () => {} });
    const link = {
      id: 'edge-1',
      from: 'a',
      to: 'b',
      points: [
        { x: 0, y: 0, z: 0 },
        { x: 10, y: 0, z: 20 },
        { x: 20, y: 0, z: 20 },
        { x: 30, y: 0, z: 0 },
      ],
      color: '#71717a',
      radius: 0.55,
      kind: 'edge' as const,
    };
    scene.setSpec({ ...emptySpec([]), links: [link] });
    const tubes: Mesh[] = [];
    scene['content'].traverse((obj) => {
      if (obj instanceof Mesh && obj.geometry instanceof TubeGeometry) tubes.push(obj);
    });
    expect(tubes).toHaveLength(1);
    expect(tubes[0].userData.pick).toEqual({ type: 'link', link });
    expect(scene['pickables']).toContain(tubes[0]);
    scene.dispose();
  });
});

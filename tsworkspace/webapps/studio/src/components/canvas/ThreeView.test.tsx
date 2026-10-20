import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({ webgl: true, loads: 0 }));

vi.mock('@/lib/webgl', () => ({ hasWebGL: () => state.webgl }));
// The factory runs only when ThreeView imports the board, so `loads` counts real loads of the 3D code.
vi.mock('@/components/canvas/three/LaneMeshBoard', () => {
  state.loads += 1;
  return { default: () => <div data-testid="lane-mesh-board" /> };
});

import { buildModel } from '@/lib/model';
import { ThreeView } from './ThreeView';

const props = { model: buildModel([]), selections: [], onSelect: () => {} };

describe('ThreeView', () => {
  beforeEach(() => {
    state.webgl = true;
  });
  afterEach(cleanup);

  it('shows a plain message and loads no 3D code when the browser has no WebGL', async () => {
    state.webgl = false;
    render(<ThreeView view="lanes" {...props} />);
    expect(screen.getByText('This view needs WebGL')).toBeDefined();
    await new Promise((r) => setTimeout(r, 20));
    expect(state.loads).toBe(0);
    expect(screen.queryByTestId('lane-mesh-board')).toBeNull();
  });

  it('loads the board on first use and shows a loading state until it arrives', async () => {
    render(<ThreeView view="lanes" {...props} />);
    expect(screen.getByText('Loading the 3D view')).toBeDefined();
    expect(await screen.findByTestId('lane-mesh-board')).toBeDefined();
    expect(state.loads).toBe(1);
    expect(screen.queryByText('Loading the 3D view')).toBeNull();
  });
});

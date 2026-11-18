import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SceneSpec } from '@/lib/scene3d';

const renderer = vi.hoisted(() => ({ webgl: true, specs: [] as SceneSpec[], disposals: 0 }));

vi.mock('@/lib/webgl', () => ({ hasWebGL: () => renderer.webgl }));
vi.mock('@/components/canvas/three/scene', () => ({
  GraphScene: class {
    setSpec(spec: SceneSpec) {
      renderer.specs.push(spec);
    }
    setFocus() {}
    setStep() {}
    dispose() {
      renderer.disposals += 1;
    }
  },
}));

import { FeaturePreviewIllustration } from './FeaturePreviewIllustration';

describe('Feature preview renderer', () => {
  beforeEach(() => {
    renderer.webgl = true;
    renderer.specs = [];
    renderer.disposals = 0;
  });
  afterEach(cleanup);

  it('renders the real Stream towers scene with event discs and linked side cards', async () => {
    render(<FeaturePreviewIllustration feature="streamTowers" />);
    await waitFor(() => expect(renderer.specs.length).toBeGreaterThan(0));
    const spec = renderer.specs.at(-1);
    expect(spec?.nodes.some((node) => node.entity.kind === 'event' && node.body.shape.type === 'cylinder')).toBe(true);
    for (const kind of ['command', 'readModel', 'processor']) {
      expect(spec?.nodes.some((node) => node.entity.kind === kind && node.body.shape.type === 'box')).toBe(true);
    }
    expect(spec?.plates.filter((plate) => plate.kicker === 'Event stream').length).toBeGreaterThan(1);
    expect(spec?.links.some((link) => link.kind === 'edge')).toBe(true);
    expect(screen.getByText('Example model')).toBeDefined();
  });

  it('uses the real Lane mesh scene and frees it when switching previews or closing', async () => {
    const { rerender, unmount } = render(<FeaturePreviewIllustration feature="laneMesh" />);
    await waitFor(() => expect(renderer.specs.length).toBeGreaterThan(0));
    const lanes = renderer.specs.at(-1);
    expect(lanes?.nodes.some((node) => node.entity.kind === 'event' && node.body.shape.type === 'box')).toBe(true);
    expect(lanes?.steps.length).toBeGreaterThan(1);
    expect(lanes?.links.some((link) => link.kind === 'edge')).toBe(true);

    rerender(<FeaturePreviewIllustration feature="streamTowers" />);
    await waitFor(() =>
      expect(renderer.specs.at(-1)?.nodes.some((node) => node.body.shape.type === 'cylinder')).toBe(true),
    );
    expect(renderer.disposals).toBe(1);
    unmount();
    expect(renderer.disposals).toBe(2);
  });

  it('reports when the real renderer cannot run without WebGL', async () => {
    renderer.webgl = false;
    render(<FeaturePreviewIllustration feature="streamTowers" />);
    expect(await screen.findByText('This view needs WebGL')).toBeDefined();
    expect(renderer.specs).toHaveLength(0);
  });

  it('renders the real State machine view without WebGL', () => {
    renderer.webgl = false;
    render(<FeaturePreviewIllustration feature="stateMachine" />);
    expect(screen.getByRole('region', { name: 'Order lifecycle' })).toBeDefined();
    expect(screen.getByRole('button', { name: 'Back to Payment authorized' })).toBeDefined();
    expect(screen.getByText('Same view, fictional data')).toBeDefined();
    expect(renderer.specs).toHaveLength(0);
  });
});

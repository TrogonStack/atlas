import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SceneEvents } from './scene';

// jsdom has no WebGL, so the renderer is replaced by a recorder. The real
// scene is drawn and checked in the browser.
const fake = vi.hoisted(() => ({
  fail: false,
  failOnSetSpec: false,
  events: undefined as SceneEvents | undefined,
  calls: [] as [string, unknown?][],
}));

vi.mock('./scene', () => ({
  GraphScene: class {
    constructor(_host: HTMLElement, events: SceneEvents) {
      if (fake.fail) throw new Error('Error creating WebGL context.');
      fake.events = events;
    }
    setSpec(_spec: unknown, pose?: unknown) {
      if (fake.failOnSetSpec) throw new Error('setSpec failed');
      fake.calls.push(['setSpec', pose]);
    }
    setFocus(focus: { selected: Set<string> }) {
      fake.calls.push(['setFocus', [...focus.selected]]);
    }
    setStep(step?: number) {
      fake.calls.push(['setStep', step]);
    }
    goTo(name: unknown) {
      fake.calls.push(['goTo', name]);
    }
    focusOn(ids: string[]) {
      fake.calls.push(['focusOn', ids]);
    }
    dispose() {
      fake.calls.push(['dispose']);
    }
  },
}));

import type { Selection } from '@/components/canvas/Board';
import { buildLaneMesh } from '@/lib/lanemesh';
import { buildModel } from '@/lib/model';
import { storefrontEntities } from '@/lib/scene3d.fixture';
import { GraphBoard } from './GraphBoard';

const model = buildModel(storefrontEntities() as never);
const spec = buildLaneMesh(model);
const calls = (name: string) => fake.calls.filter(([n]) => n === name).map(([, arg]) => arg);

describe('GraphBoard', () => {
  beforeEach(() => {
    fake.fail = false;
    fake.failOnSetSpec = false;
    fake.events = undefined;
    fake.calls = [];
  });
  afterEach(cleanup);

  it('shows a plain message when the browser cannot start WebGL', () => {
    fake.fail = true;
    render(<GraphBoard spec={spec} selections={[]} onSelect={() => {}} />);
    expect(screen.getByText('This view needs WebGL')).toBeDefined();
    expect(screen.queryByRole('button', { name: 'Front' })).toBeNull();
  });

  it('shows a message when the browser takes the graphics context back', () => {
    render(<GraphBoard spec={spec} selections={[]} onSelect={() => {}} />);
    act(() => fake.events?.lost());
    expect(screen.getByText('The 3D view stopped')).toBeDefined();
  });

  it('frees the scene when setup fails after WebGL starts', () => {
    fake.failOnSetSpec = true;
    const { unmount } = render(<GraphBoard spec={spec} selections={[]} onSelect={() => {}} />);
    expect(screen.getByText('This view needs WebGL')).toBeDefined();
    expect(calls('dispose')).toHaveLength(1);
    unmount();
    expect(calls('dispose')).toHaveLength(2);
  });

  it('opens on the overview and frees the scene on unmount', () => {
    const { unmount } = render(<GraphBoard spec={spec} selections={[]} onSelect={() => {}} />);
    expect(calls('setSpec')).toEqual(['overview']);
    unmount();
    expect(calls('dispose')).toHaveLength(1);
  });

  it('passes a click on a card to onSelect with its Board occurrence', () => {
    const onSelect = vi.fn();
    render(<GraphBoard spec={spec} selections={[]} onSelect={onSelect} />);
    const node = spec.nodes[0];
    act(() => fake.events?.pick({ type: 'node', node }, true));
    expect(onSelect).toHaveBeenCalledWith(node.entity.key, node.occId, true, node.instanceUid, node.momentKey);
    act(() => fake.events?.pick(undefined, false));
    expect(onSelect).toHaveBeenLastCalledWith(undefined);
  });

  it('keeps the selection when a code file is clicked', () => {
    const onSelect = vi.fn();
    render(<GraphBoard spec={spec} selections={[]} onSelect={onSelect} />);
    const file = {
      id: 'file:x',
      body: { id: 'file:x', center: { x: 0, y: 0, z: 0 }, shape: { type: 'sphere' as const, radius: 1 } },
      repo: 'storefront',
      path: 'lib/x.ex',
      symbols: [],
      citedBy: [],
    };
    act(() => fake.events?.pick({ type: 'file', file }, false));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('shows the selection in the scene', () => {
    const node = spec.nodes[0];
    const selections: Selection[] = [{ type: 'entity', key: node.entity.key, occId: node.id }];
    const { rerender } = render(<GraphBoard spec={spec} selections={[]} onSelect={() => {}} />);
    rerender(<GraphBoard spec={spec} selections={selections} onSelect={() => {}} />);
    expect(calls('setFocus').at(-1)).toEqual([node.id]);
  });

  it('flies to the card a focus request names', () => {
    const node = spec.nodes.find((n) => n.entity.id.slug === 'place-order' && n.entity.kind === 'command');
    if (!node) throw new Error('no place-order card');
    const { rerender } = render(<GraphBoard spec={spec} selections={[]} onSelect={() => {}} />);
    rerender(<GraphBoard spec={spec} selections={[]} onSelect={() => {}} focus={{ key: node.entity.key, n: 1 }} />);
    expect(calls('focusOn')).toEqual([[node.id]]);
  });

  it('moves the camera to the presets and steps through the slices', () => {
    render(<GraphBoard spec={spec} selections={[]} onSelect={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: 'Front' }));
    fireEvent.click(screen.getByRole('button', { name: 'Overview' }));
    fireEvent.click(screen.getByRole('button', { name: 'Next slice' }));
    expect(screen.getByText(`Slice 1 of ${spec.steps.length}: ${spec.steps[0].label}`)).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Next slice' }));
    fireEvent.click(screen.getByRole('button', { name: 'Previous slice' }));
    expect(calls('goTo')).toEqual(['front', 'overview', { step: 0 }, { step: 1 }, { step: 0 }]);
    expect(calls('setStep').at(-1)).toBe(0);
  });

  it('says so when the model has nothing to draw', () => {
    render(<GraphBoard spec={buildLaneMesh(buildModel([]))} selections={[]} onSelect={() => {}} />);
    expect(screen.getByText('Nothing to draw yet')).toBeDefined();
    expect(screen.queryByRole('button', { name: 'Next slice' })).toBeNull();
  });
});

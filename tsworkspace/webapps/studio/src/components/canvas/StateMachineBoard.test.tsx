import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Selection } from '@/components/canvas/Board';
import { buildStateMachinePreviewModel } from '@/lib/feature-preview-model';
import { buildModel, type EntityId, entityKey } from '@/lib/model';
import { StateMachineBoard } from './StateMachineBoard';

const ns = 'preview-storefront';
const eventKey = (slug: string) => entityKey('event', { namespace: ns, slug, version: '1' });
const state = (slug: string) => {
  const el = document.querySelector(`[data-state="${slug}"]`);
  if (!el) throw new Error(`no state ${slug}`);
  return el as HTMLButtonElement;
};

const scrollIntoView = vi.fn();
const originalScrollIntoView = Element.prototype.scrollIntoView;

describe('StateMachineBoard', () => {
  beforeEach(() => {
    scrollIntoView.mockReset();
    vi.stubGlobal(
      'ResizeObserver',
      class {
        constructor(private callback: ResizeObserverCallback) {}
        observe(target: Element) {
          this.callback(
            [{ target, contentRect: { width: 800, height: 400 } } as ResizeObserverEntry],
            this as unknown as ResizeObserver,
          );
        }
        unobserve() {}
        disconnect() {}
      },
    );
    Element.prototype.scrollIntoView = scrollIntoView;
  });

  afterEach(() => {
    cleanup();
    Element.prototype.scrollIntoView = originalScrollIntoView;
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('draws every state of the lifecycle with its kind', () => {
    render(<StateMachineBoard model={buildStateMachinePreviewModel()} selections={[]} onSelect={vi.fn()} />);
    expect(screen.getByRole('region', { name: 'Order lifecycle' })).toBeDefined();
    expect(state('order.placed').dataset.kind).toBe('row');
    expect(state('order.delivered').dataset.kind).toBe('final');
    expect(screen.getByRole('button', { name: 'Order delivered: final, nothing comes next' })).toBeDefined();
    expect(screen.getByRole('button', { name: 'Back to Payment authorized' })).toBeDefined();
    expect(screen.getByText('6 states · 6 transitions · 1 return')).toBeDefined();
  });

  it('selects the event behind a state or a stub', () => {
    const onSelect = vi.fn();
    render(<StateMachineBoard model={buildStateMachinePreviewModel()} selections={[]} onSelect={onSelect} />);
    fireEvent.click(state('order.placed'));
    expect(onSelect).toHaveBeenLastCalledWith(eventKey('order.placed'), undefined, false);
    fireEvent.click(screen.getByRole('button', { name: 'Back to Payment authorized' }));
    expect(onSelect).toHaveBeenLastCalledWith(eventKey('payment.authorized'), undefined, false);
  });

  it('lists the guards of the selected state in their table order', () => {
    const selections: Selection[] = [{ type: 'entity', key: eventKey('payment.authorized') }];
    render(<StateMachineBoard model={buildStateMachinePreviewModel()} selections={selections} onSelect={vi.fn()} />);
    const next = within(screen.getByTestId('state-machine-next'));
    expect(next.getByText('After Payment authorized: the table lists what comes next')).toBeDefined();
    expect(next.getAllByRole('listitem').map((li) => li.textContent)).toEqual([
      '1Order shipped The warehouse ships before the authorization lapses.',
      '2Payment voided The authorization lapses before shipping.',
    ]);
    expect(state('payment.authorized').getAttribute('aria-pressed')).toBe('true');
  });

  it('switches streams and keeps a missing event visible', () => {
    const id = (slug: string): EntityId => ({ namespace: 'acme', slug, version: '1' });
    const ref = (slug: string) => ({ id: id(slug) });
    const model = buildModel([
      {
        swimlane: {
          id: id('cart'),
          title: 'Cart',
          order: 1,
          transitions: [{ after: ref('cart.opened'), next: [{ event: ref('cart.checked-out') }] }],
        },
      },
      {
        swimlane: {
          id: id('refund'),
          title: 'Refund',
          order: 2,
          transitions: [{ after: ref('refund.requested'), next: [{ event: ref('refund.ghost') }] }],
        },
      },
      { event: { id: id('cart.opened'), title: 'Cart opened', swimlane: ref('cart') } },
      { event: { id: id('cart.checked-out'), title: 'Cart checked out', swimlane: ref('cart') } },
      { event: { id: id('cart.note-added'), title: 'Note added', swimlane: ref('cart') } },
      { event: { id: id('refund.requested'), title: 'Refund requested', swimlane: ref('refund') } },
    ]);
    render(<StateMachineBoard model={model} selections={[]} onSelect={vi.fn()} />);
    expect(screen.getByText('1 event on this stream is in no transition:', { exact: false })).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Refund' }));
    expect(screen.getByRole('region', { name: 'Refund lifecycle' })).toBeDefined();
    expect(state('refund.ghost').dataset.problem).toBe('missing');
    expect(state('refund.ghost').disabled).toBe(false);
  });

  it('reveals a focused state again on each request without repanning on model refresh', () => {
    const key = eventKey('order.delivered');
    const onSelect = vi.fn();
    const { rerender } = render(
      <StateMachineBoard
        model={buildStateMachinePreviewModel()}
        selections={[]}
        onSelect={onSelect}
        focus={{ key, n: 1 }}
      />,
    );
    expect(scrollIntoView.mock.contexts).toEqual([state('order.delivered')]);
    scrollIntoView.mockClear();
    rerender(
      <StateMachineBoard
        model={buildStateMachinePreviewModel()}
        selections={[]}
        onSelect={onSelect}
        focus={{ key, n: 1 }}
      />,
    );
    expect(scrollIntoView).not.toHaveBeenCalled();
    rerender(
      <StateMachineBoard
        model={buildStateMachinePreviewModel()}
        selections={[]}
        onSelect={onSelect}
        focus={{ key, n: 2 }}
      />,
    );
    expect(scrollIntoView.mock.contexts).toEqual([state('order.delivered')]);
  });

  it('reveals the target after switching to its stream', () => {
    const id = (slug: string): EntityId => ({ namespace: 'acme', slug, version: '1' });
    const ref = (slug: string) => ({ id: id(slug) });
    const model = buildModel([
      { swimlane: { id: id('cart'), title: 'Cart', order: 1, transitions: [{ after: ref('cart.opened'), next: [] }] } },
      {
        swimlane: {
          id: id('refund'),
          title: 'Refund',
          order: 2,
          transitions: [{ after: ref('refund.requested'), next: [] }],
        },
      },
      { event: { id: id('cart.opened'), title: 'Cart opened', swimlane: ref('cart') } },
      { event: { id: id('refund.requested'), title: 'Refund requested', swimlane: ref('refund') } },
    ]);
    const onSelect = vi.fn();
    const { rerender } = render(<StateMachineBoard model={model} selections={[]} onSelect={onSelect} />);
    const key = entityKey('event', id('refund.requested'));
    rerender(<StateMachineBoard model={model} selections={[]} onSelect={onSelect} focus={{ key, n: 1 }} />);
    expect(screen.getByRole('region', { name: 'Refund lifecycle' })).toBeDefined();
    expect(scrollIntoView.mock.contexts).toEqual([state('refund.requested')]);
  });

  it('keeps a tall lifecycle readable and scrollable while thumbnails can fit the whole drawing', () => {
    const id = (slug: string): EntityId => ({ namespace: 'acme', slug, version: '1' });
    const ref = (slug: string) => ({ id: id(slug) });
    const successors = Array.from({ length: 12 }, (_, i) => `closed-${i}`);
    const model = buildModel([
      {
        swimlane: {
          id: id('fan-out'),
          title: 'Fan out',
          transitions: [
            { after: ref('opened'), next: successors.map((slug) => ({ event: ref(slug) })) },
            ...successors.map((slug) => ({ after: ref(slug), next: [] })),
          ],
        },
      },
      ...['opened', ...successors].map((slug) => ({ event: { id: id(slug), title: slug, swimlane: ref('fan-out') } })),
    ]);
    const { rerender } = render(<StateMachineBoard model={model} selections={[]} onSelect={vi.fn()} />);
    const scale = () => Number(state('opened').parentElement?.style.transform.match(/scale\(([^)]+)\)/)?.[1]);
    expect(scale()).toBeGreaterThanOrEqual(0.8);
    expect(screen.getByRole('region', { name: 'Fan out lifecycle' }).parentElement?.className).toContain(
      'overflow-auto',
    );
    rerender(<StateMachineBoard model={model} selections={[]} onSelect={vi.fn()} fitWhole />);
    expect(scale()).toBeLessThan(0.8);
  });

  it('lets a missing state and its return stub expose guards without selecting an absent entity', () => {
    const id = (slug: string): EntityId => ({ namespace: 'acme', slug, version: '1' });
    const ref = (slug: string) => ({ id: id(slug) });
    const model = buildModel([
      {
        swimlane: {
          id: id('refund'),
          title: 'Refund',
          transitions: [
            { after: ref('refund.ghost'), next: [{ event: ref('refund.requested'), doc: 'A refund was requested.' }] },
            { after: ref('refund.requested'), next: [{ event: ref('refund.ghost'), doc: 'The refund is retried.' }] },
          ],
        },
      },
      { event: { id: id('refund.requested'), title: 'Refund requested', swimlane: ref('refund') } },
    ]);
    const onSelect = vi.fn();
    const { rerender } = render(<StateMachineBoard model={model} selections={[]} onSelect={onSelect} />);
    fireEvent.click(state('refund.ghost'));
    expect(within(screen.getByTestId('state-machine-next')).getByText('A refund was requested.')).toBeDefined();
    expect(state('refund.ghost').getAttribute('aria-pressed')).toBe('true');
    expect(onSelect).not.toHaveBeenCalled();
    const selections: Selection[] = [{ type: 'entity', key: entityKey('event', id('refund.requested')) }];
    rerender(<StateMachineBoard model={model} selections={selections} onSelect={onSelect} />);
    expect(state('refund.requested').getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'Back to refund.ghost' }));
    expect(within(screen.getByTestId('state-machine-next')).getByText('A refund was requested.')).toBeDefined();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('shows an empty state when no stream declares transitions', () => {
    render(<StateMachineBoard model={buildModel([])} selections={[]} onSelect={vi.fn()} />);
    expect(screen.getByText('No lifecycle to draw')).toBeDefined();
  });
});

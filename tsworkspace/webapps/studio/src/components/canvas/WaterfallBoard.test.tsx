import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Selection } from '@/components/canvas/Board';
import { IssuesProvider } from '@/components/IssuesProvider';
import { buildIssueIndex } from '@/lib/issues';
import { buildModel, type Model } from '@/lib/model';
import type { FocusRequest } from './useFocusNode';
import { WaterfallBoard } from './WaterfallBoard';

const NS = 'acme-storefront';
const id = (slug: string) => ({ namespace: NS, slug, version: '1' });
const ref = (slug: string) => ({ id: id(slug) });
const key = (kind: string, slug: string) => `${kind}:${NS}/${slug}@1`;

// Checkout places an order. Payment reads the pending payments view that
// Checkout projects, so Payment joins Checkout there.
function storefront(opts: { code?: boolean } = {}): Model {
  return buildModel([
    { persona: { id: id('shopper'), title: 'Shopper' } },
    { swimlane: { id: id('order'), title: 'Order', order: 1 } },
    { ui: { id: id('storefront-checkout'), title: 'Checkout page' } },
    {
      command: {
        id: id('place-order'),
        title: 'Place order',
        metadata: opts.code
          ? [
              {
                '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.CodeRefAnnotation',
                repo: 'storefront',
                path: 'services/checkout/place_order.go',
              },
            ]
          : [],
      },
    },
    { command: { id: id('capture-payment'), title: 'Capture payment' } },
    { event: { id: id('order.placed'), title: 'Order placed', swimlane: ref('order') } },
    { event: { id: id('payment.captured'), title: 'Payment captured', swimlane: ref('order') } },
    { readModel: { id: id('pending-payments'), title: 'Pending payments' } },
    { processor: { id: id('payment-capturer'), title: 'Payment capturer' } },
    {
      commandSlice: {
        id: id('place-order-slice'),
        title: 'Place order',
        persona: { persona: ref('shopper') },
        ui: { ui: ref('storefront-checkout') },
        command: { command: ref('place-order') },
        emittedEvents: [{ event: ref('order.placed') }],
      },
    },
    {
      readModelSlice: {
        id: id('pending-payments-rms'),
        title: 'Pending payments',
        readModel: { readModel: ref('pending-payments') },
        sourceEvents: [{ event: ref('order.placed') }],
      },
    },
    {
      automationSlice: {
        id: id('capture-payment-auto'),
        title: 'Capture payment',
        processor: { processor: ref('payment-capturer') },
        emittedCommand: { command: ref('capture-payment') },
        sourceReadModels: [{ readModel: ref('pending-payments') }],
      },
    },
    {
      commandSlice: {
        id: id('capture-payment-slice'),
        title: 'Capture payment',
        command: { command: ref('capture-payment') },
        emittedEvents: [{ event: ref('payment.captured') }],
      },
    },
    {
      storyboard: {
        id: id('checkout-flow'),
        title: 'Checkout',
        slices: [ref('place-order-slice'), ref('pending-payments-rms')],
      },
    },
    {
      storyboard: {
        id: id('payment-flow'),
        title: 'Payment',
        slices: [ref('capture-payment-auto'), ref('capture-payment-slice')],
      },
    },
  ] as never);
}

const scrollIntoView = vi.fn();
const originalScrollIntoView = Element.prototype.scrollIntoView;

beforeEach(() => {
  scrollIntoView.mockClear();
  // jsdom does not lay out, so it has no scrollIntoView.
  Element.prototype.scrollIntoView = scrollIntoView;
});

afterEach(cleanup);

afterAll(() => {
  Element.prototype.scrollIntoView = originalScrollIntoView;
});

function renderBoard(
  props: { model?: Model; selections?: Selection[]; focus?: FocusRequest; onSelect?: () => void } = {},
) {
  const onSelect = props.onSelect ?? vi.fn();
  const view = (p: typeof props) => (
    <WaterfallBoard
      model={p.model ?? storefront()}
      selections={p.selections ?? []}
      onSelect={onSelect}
      focus={p.focus}
    />
  );
  const result = render(view(props));
  return { ...result, onSelect, rerenderWith: (next: typeof props) => result.rerender(view({ ...props, ...next })) };
}

const rowsOf = (type: string) => document.querySelectorAll(`[data-row-type="${type}"]`);
const rowFor = (entityKey: string, type = 'step') =>
  document.querySelector<HTMLElement>(`[data-row-type="${type}"][data-entity-key="${entityKey}"]`);

describe('WaterfallBoard', () => {
  it('lists one closed group per storyboard', () => {
    renderBoard();
    expect(screen.getByText('Checkout')).toBeDefined();
    expect(screen.getByText('Payment')).toBeDefined();
    expect(rowsOf('group')).toHaveLength(2);
    expect(rowsOf('step')).toHaveLength(0);
    expect(screen.getByText('2 storyboards · 7 steps · 1 shared')).toBeDefined();
    // One header column per slice.
    expect(screen.getByTitle('Slice 4: Capture payment')).toBeDefined();
  });

  it('opens and closes a group', () => {
    renderBoard();
    fireEvent.click(screen.getByRole('button', { name: 'Open Checkout' }));
    expect(screen.getByRole('button', { name: 'Close Checkout' }).getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByText('Place order')).toBeDefined();
    expect(rowsOf('step')).toHaveLength(4);
    fireEvent.click(screen.getByRole('button', { name: 'Close Checkout' }));
    expect(rowsOf('step')).toHaveLength(0);
  });

  it('opens and closes every group at once', () => {
    renderBoard();
    fireEvent.click(screen.getByRole('button', { name: 'Open all' }));
    expect(rowsOf('step')).toHaveLength(7);
    expect(rowsOf('join')).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: 'Close all' }));
    expect(rowsOf('step')).toHaveLength(0);
  });

  it('shows the kind, the title and the Board lane on each row', () => {
    renderBoard();
    fireEvent.click(screen.getByRole('button', { name: 'Open all' }));
    const row = rowFor(key('event', 'order.placed'));
    expect(row).not.toBeNull();
    const cells = within(row as HTMLElement);
    expect(cells.getByText('Event')).toBeDefined();
    expect(cells.getByText('Order placed')).toBeDefined();
    expect(cells.getByText('Order')).toBeDefined();
    // The processor's kind and its Board lane are both called Automation.
    expect(within(rowFor(key('processor', 'payment-capturer')) as HTMLElement).getAllByText('Automation')).toHaveLength(
      2,
    );
  });

  it('selects the entity when its row is clicked', () => {
    const { onSelect } = renderBoard();
    fireEvent.click(screen.getByRole('button', { name: 'Open Checkout' }));
    fireEvent.click(screen.getByRole('button', { name: 'Place order' }));
    expect(onSelect).toHaveBeenCalledWith(key('command', 'place-order'), undefined, false);
    fireEvent.click(screen.getByRole('button', { name: 'Order placed' }), { metaKey: true });
    expect(onSelect).toHaveBeenLastCalledWith(key('event', 'order.placed'), undefined, true);
  });

  it('selects the storyboard from its group header', () => {
    const { onSelect } = renderBoard();
    fireEvent.click(screen.getByRole('button', { name: 'Payment' }));
    expect(onSelect).toHaveBeenCalledWith(key('storyboard', 'payment-flow'), undefined, false);
  });

  it('jumps from a join row to the row that owns the step', () => {
    const { onSelect } = renderBoard();
    fireEvent.click(screen.getByRole('button', { name: 'Open Payment' }));
    const join = rowFor(key('readModel', 'pending-payments'), 'join');
    expect(join).not.toBeNull();
    expect(within(join as HTMLElement).getByText('joins Checkout')).toBeDefined();
    fireEvent.click(within(join as HTMLElement).getByRole('button', { name: 'Pending payments' }));
    expect(onSelect).toHaveBeenCalledWith(key('readModel', 'pending-payments'), undefined, false);
    // The owner's group opens and its row scrolls into view.
    expect(screen.getByRole('button', { name: 'Close Checkout' })).toBeDefined();
    const owner = rowFor(key('readModel', 'pending-payments'));
    expect(scrollIntoView.mock.contexts).toContain(owner);
  });

  it('marks rows that hold a selected entity', () => {
    renderBoard({ selections: [{ type: 'entity', key: key('readModel', 'pending-payments') }] });
    fireEvent.click(screen.getByRole('button', { name: 'Open all' }));
    expect(rowFor(key('readModel', 'pending-payments'))?.dataset.selected).toBe('true');
    expect(rowFor(key('readModel', 'pending-payments'), 'join')?.dataset.selected).toBe('true');
    expect(rowFor(key('command', 'place-order'))?.dataset.selected).toBeUndefined();
    // The header lights the slice where the selection occurs.
    expect(screen.getByTitle('Slice 2: Pending payments').className).toContain('bg-foreground');
  });

  it('opens the group of a focused entity and scrolls its row into view', () => {
    const { rerenderWith } = renderBoard();
    expect(rowsOf('step')).toHaveLength(0);
    rerenderWith({ focus: { key: key('event', 'payment.captured'), n: 1 } });
    const row = rowFor(key('event', 'payment.captured'));
    expect(row).not.toBeNull();
    expect(scrollIntoView.mock.contexts).toContain(row);
  });

  it('groups by lane and kind and draws cables between the groups', () => {
    renderBoard();
    fireEvent.click(screen.getByRole('button', { name: 'By object type' }));
    expect(screen.getByRole('button', { name: 'By object type' }).getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByText('Commands')).toBeDefined();
    expect(screen.getByText('Read models')).toBeDefined();
    expect(screen.getByTestId('waterfall-cables').querySelectorAll('[data-cable]')).toHaveLength(5);
    const cable = screen.getByRole('button', { name: /^2 links from Commands \(Timeline\) to Events \(Order\)/ });
    expect(cable.textContent).toBe('2');
    // A cable's count opens both of its ends.
    fireEvent.click(cable);
    expect(screen.getByRole('button', { name: 'Close Commands' })).toBeDefined();
    expect(screen.getByRole('button', { name: 'Close Events' })).toBeDefined();
    expect(rowFor(key('command', 'capture-payment'))).not.toBeNull();
    expect(within(rowFor(key('command', 'capture-payment')) as HTMLElement).getByText('1 out · 1 in')).toBeDefined();
  });

  it('keeps the selection in view when the grouping changes', () => {
    renderBoard({ selections: [{ type: 'entity', key: key('event', 'order.placed') }] });
    fireEvent.click(screen.getByRole('button', { name: 'By object type' }));
    const row = rowFor(key('event', 'order.placed'));
    expect(row).not.toBeNull();
    expect(scrollIntoView.mock.contexts).toContain(row);
  });

  it('shows issue counts from the validator on rows and group headers', () => {
    const issues = buildIssueIndex([
      {
        severity: 'SEVERITY_WARNING',
        code: 'RM_EVENT_NOT_PROJECTED',
        message: 'Pending payments declares an event no slice projects.',
        subject: { kind: 'ENTITY_KIND_READ_MODEL', id: id('pending-payments') },
      },
    ]);
    render(
      <IssuesProvider issues={issues}>
        <WaterfallBoard model={storefront()} selections={[]} onSelect={vi.fn()} />
      </IssuesProvider>,
    );
    const header = document.querySelector<HTMLElement>('[data-row-type="group"]');
    expect(header?.querySelector('[data-severity="warning"]')).not.toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Open Checkout' }));
    const row = rowFor(key('readModel', 'pending-payments'));
    expect(row?.querySelector('[data-severity="warning"]')?.textContent).toBe('1');
  });

  it('hides the code column when no entity carries a code reference', () => {
    renderBoard();
    expect(screen.queryByText('Code, by folder')).toBeNull();
  });

  it('shows code chips bundled by folder when entities carry code references', () => {
    renderBoard({ model: storefront({ code: true }) });
    expect(screen.getByText('Code, by folder')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Open Checkout' }));
    const row = rowFor(key('command', 'place-order'));
    const chip = within(row as HTMLElement).getByText('services/checkout');
    expect(chip.parentElement?.title).toContain('storefront/services/checkout');
  });
});

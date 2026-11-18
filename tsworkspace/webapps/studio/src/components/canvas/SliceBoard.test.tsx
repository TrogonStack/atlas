import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { withNuqsTestingAdapter } from 'nuqs/adapters/testing';
import { afterEach, describe, expect, it, type Mock, vi } from 'vitest';
import { IssuesProvider } from '@/components/IssuesProvider';
import { buildIssueIndex, EMPTY_ISSUE_INDEX, type IssueIndex } from '@/lib/issues';
import { buildModel, entityKey, type Model } from '@/lib/model';
import { id, storefront } from '@/lib/sliceview.fixture';
import { PLAY_STEP_MS, SliceBoard } from './SliceBoard';

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

type OnSelect = Parameters<typeof SliceBoard>[0]['onSelect'];

const model = storefront();
const key = (kind: Parameters<typeof entityKey>[0], slug: string) => entityKey(kind, id(slug));

function renderBoard({
  m = model,
  slice,
  selections = [],
  issues = EMPTY_ISSUE_INDEX,
  onSelect = vi.fn<OnSelect>(),
}: {
  m?: Model;
  slice?: string;
  selections?: { type: 'entity'; key: string }[];
  issues?: IssueIndex;
  onSelect?: Mock<OnSelect>;
} = {}) {
  const onUrlUpdate = vi.fn();
  const view = render(
    <IssuesProvider issues={issues}>
      <SliceBoard model={m} selections={selections} onSelect={onSelect} />
    </IssuesProvider>,
    {
      wrapper: withNuqsTestingAdapter({
        searchParams: slice ? { slice: `acme-storefront/${slice}` } : undefined,
        hasMemory: true,
        onUrlUpdate,
      }),
    },
  );
  const lastSlice = () => onUrlUpdate.mock.calls.at(-1)?.[0].searchParams.get('slice');
  return { ...view, onSelect, onUrlUpdate, lastSlice };
}

const card = (name: string) => screen.getByRole('button', { name });
const picker = () => screen.getByRole('combobox', { name: 'Slice' }) as HTMLSelectElement;

describe('SliceBoard picking a slice', () => {
  it('opens the first slice in story order when nothing is selected', () => {
    renderBoard();
    expect(picker().value).toBe('acme-storefront/add-item-to-cart-slice');
    expect(screen.getByRole('button', { name: 'Slice: Add item to cart' })).toBeDefined();
  });

  it('opens the slice of the entity selected when the view opens', () => {
    renderBoard({ selections: [{ type: 'entity', key: key('event', 'payment.captured') }] });
    expect(picker().value).toBe('acme-storefront/capture-payment-slice');
  });

  it('groups the picker by storyboard and keeps the pick in the URL', async () => {
    const { lastSlice } = renderBoard();
    const groups = [...picker().querySelectorAll('optgroup')].map((g) => g.label);
    expect(groups).toEqual(['Checkout', 'Fulfillment']);
    fireEvent.change(picker(), { target: { value: 'acme-storefront/place-order-slice' } });
    await waitFor(() => expect(lastSlice()).toBe('acme-storefront/place-order-slice'));
    expect(screen.getByRole('button', { name: 'Slice: Place order' })).toBeDefined();
    expect(card('Read Model: Pending payments')).toBeDefined();
  });

  it('reads the slice from the URL and draws its neighbors', () => {
    renderBoard({ slice: 'place-order-slice' });
    expect(card('UI: Checkout')).toBeDefined();
    expect(card('Command: Place order')).toBeDefined();
    expect(card('Event: Order placed')).toBeDefined();
    expect(screen.getByText('Downstream')).toBeDefined();
    expect(card('Read Model: Order status')).toBeDefined();
  });

  it('clears a ?slice= value that names no slice and shows the fallback slice', async () => {
    const { lastSlice, onUrlUpdate } = renderBoard({ slice: 'gone' });
    await waitFor(() => expect(onUrlUpdate).toHaveBeenCalled());
    expect(lastSlice()).toBeNull();
    expect(picker().value).toBe('acme-storefront/add-item-to-cart-slice');
  });

  it('says why the board is empty when the model has no slices', () => {
    const empty = buildModel([{ event: { id: id('order.placed'), title: 'Order placed' } }] as never);
    renderBoard({ m: empty });
    expect(screen.getByText('No slice to show')).toBeDefined();
  });
});

describe('SliceBoard field flow', () => {
  it('lights the cards that carry the field, dims the rest and marks where it stops', () => {
    renderBoard({ slice: 'place-order-slice' });
    fireEvent.click(screen.getByRole('button', { name: 'cart_id' }));
    expect(screen.getByRole('button', { name: 'cart_id' }).getAttribute('aria-pressed')).toBe('true');
    expect(card('Command: Place order').dataset.emphasis).toBe('lit');
    expect(card('Event: Order placed').dataset.emphasis).toBe('lit');
    // Nothing flows into the screen, so the field does not pass through it.
    expect(card('UI: Checkout').dataset.emphasis).toBe('dim');
    expect(card('Read Model: Pending payments').dataset.emphasis).toBe('dim');
    const stops = screen.getAllByTestId('field-stop');
    expect(stops).toHaveLength(2);
    expect(stops.map((s) => s.textContent)).toContain(
      'cart_id stops here: Order placed has it, Pending payments does not',
    );
    expect(screen.getByText(/stops at 2 links/)).toBeDefined();

    fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
    expect(card('Command: Place order').dataset.emphasis).toBeUndefined();
    expect(screen.queryAllByTestId('field-stop')).toHaveLength(0);
  });

  it('keeps the traced field when the slice changes', () => {
    renderBoard({ slice: 'place-order-slice' });
    fireEvent.click(screen.getByRole('button', { name: 'order_id' }));
    fireEvent.change(picker(), { target: { value: 'acme-storefront/capture-payment-slice' } });
    expect(card('Command: Capture payment').dataset.emphasis).toBe('lit');
    expect(card('Event: Payment captured').dataset.emphasis).toBe('lit');
  });
});

describe('SliceBoard validation findings', () => {
  const finding = (kind: string, slug: string, code: string, severity = 'SEVERITY_WARNING') => ({
    severity,
    code,
    message: `${code} on ${slug}`,
    subject: { kind, id: id(slug) },
  });

  it('lists the findings of the slice and its members, each with a fix prompt', () => {
    const issues = buildIssueIndex([
      finding('ENTITY_KIND_COMMAND_SLICE', 'place-order-slice', 'EVENT_FIELD_UNSOURCED', 'SEVERITY_ERROR'),
      finding('ENTITY_KIND_COMMAND', 'place-order', 'COMMAND_FIELD_NEVER_RECORDED'),
      // A neighbor's finding belongs to its own slice, not to this one.
      finding('ENTITY_KIND_READ_MODEL', 'pending-payments', 'RM_SOURCE_EVENT_FIELD_NOT_PROJECTED'),
    ]);
    renderBoard({ slice: 'place-order-slice', issues });
    fireEvent.click(screen.getByRole('tab', { name: /Validation/ }));
    const groups = screen.getAllByRole('region', { name: /^Findings for / });
    expect(groups).toHaveLength(2);
    expect(within(groups[0]).getByText('EVENT_FIELD_UNSOURCED on place-order-slice')).toBeDefined();
    expect(within(groups[1]).getByText('COMMAND_FIELD_NEVER_RECORDED on place-order')).toBeDefined();
    expect(screen.getAllByRole('button', { name: /Copy fix prompt/ })).toHaveLength(2);
    expect(screen.queryByText(/on pending-payments/)).toBeNull();
  });

  it('says when validation did not run instead of reporting a clean slice', () => {
    renderBoard({ slice: 'place-order-slice', issues: buildIssueIndex([], 'validator offline') });
    fireEvent.click(screen.getByRole('tab', { name: /Validation/ }));
    expect(screen.getByText(/Validation unavailable \(validator offline\)/)).toBeDefined();
  });

  it('says when the slice has no findings', () => {
    renderBoard({ slice: 'place-order-slice', issues: buildIssueIndex([]) });
    fireEvent.click(screen.getByRole('tab', { name: /Validation/ }));
    expect(screen.getByText(/No validation findings for this slice/)).toBeDefined();
  });
});

describe('SliceBoard story playback', () => {
  it('steps to the previous and next slice of the storyboard', async () => {
    const { lastSlice } = renderBoard({ slice: 'place-order-slice' });
    fireEvent.click(screen.getByRole('tab', { name: 'Story' }));
    expect(screen.getByText('3 of 5')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Next slice' }));
    await waitFor(() => expect(lastSlice()).toBe('acme-storefront/pending-payments-rms'));
    expect(screen.getByText('4 of 5')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Previous slice' }));
    fireEvent.click(screen.getByRole('button', { name: 'Previous slice' }));
    await waitFor(() => expect(lastSlice()).toBe('acme-storefront/cart-summary-rms'));
    expect(screen.getByText('2 of 5')).toBeDefined();
  });

  it('plays through the storyboard and stops on the last slice', () => {
    vi.useFakeTimers();
    renderBoard({ slice: 'pending-payments-rms' });
    fireEvent.click(screen.getByRole('tab', { name: 'Story' }));
    fireEvent.click(screen.getByRole('button', { name: 'Play' }));
    expect(screen.getByRole('button', { name: 'Pause' })).toBeDefined();
    act(() => vi.advanceTimersByTime(PLAY_STEP_MS));
    expect(screen.getByText('5 of 5')).toBeDefined();
    expect(picker().value).toBe('acme-storefront/order-status-placed-rms');
    act(() => vi.advanceTimersByTime(PLAY_STEP_MS));
    expect(screen.getByRole('button', { name: 'Play' })).toBeDefined();
    expect(screen.getByText('5 of 5')).toBeDefined();
  });

  it('restarts from the first slice when Play is pressed on the last', async () => {
    const { lastSlice } = renderBoard({ slice: 'order-status-placed-rms' });
    fireEvent.click(screen.getByRole('tab', { name: 'Story' }));
    expect(screen.getByText('5 of 5')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Play' }));
    // Must stay on Pause while go(0) lands; pausing immediately was the bug.
    expect(screen.getByRole('button', { name: 'Pause' })).toBeDefined();
    await waitFor(() => expect(lastSlice()).toBe('acme-storefront/add-item-to-cart-slice'));
    expect(screen.getByText('1 of 5')).toBeDefined();
    expect(screen.getByRole('button', { name: 'Pause' })).toBeDefined();
  });

  it('says when no storyboard lists the slice', () => {
    const loose = storefront([
      { commandSlice: { id: id('loose-slice'), title: 'Loose', command: { command: { id: id('place-order') } } } },
    ]);
    renderBoard({ m: loose, slice: 'loose-slice' });
    fireEvent.click(screen.getByRole('tab', { name: 'Story' }));
    expect(screen.getByText(/No storyboard lists this slice/)).toBeDefined();
  });
});

describe('SliceBoard scenario playback', () => {
  it('steps through given, when and then and lights the entities each step names', () => {
    renderBoard({ slice: 'place-order-slice' });
    fireEvent.click(screen.getByRole('tab', { name: /Scenarios/ }));
    fireEvent.click(
      within(screen.getByRole('list', { name: 'Scenarios' })).getByRole('button', { name: /Order placed/ }),
    );
    const given = screen.getByRole('region', { name: 'Given' });
    // The given event belongs to an earlier slice, so it is not drawn here.
    expect(within(given).getByText('not on this board')).toBeDefined();

    fireEvent.click(screen.getByRole('button', { name: 'Next step' }));
    expect(screen.getByRole('region', { name: 'When' })).toBeDefined();
    expect(card('Command: Place order').dataset.emphasis).toBe('lit');
    expect(card('Event: Order placed').dataset.emphasis).toBe('dim');

    fireEvent.click(screen.getByRole('button', { name: 'Next step' }));
    expect(card('Event: Order placed').dataset.emphasis).toBe('lit');
    expect(card('Command: Place order').dataset.emphasis).toBe('dim');
  });

  it('shows the reason a rejected command gives', () => {
    renderBoard({ slice: 'place-order-slice' });
    fireEvent.click(screen.getByRole('tab', { name: /Scenarios/ }));
    fireEvent.click(
      within(screen.getByRole('list', { name: 'Scenarios' })).getByRole('button', { name: /Empty cart/ }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Then' }));
    const then = screen.getByRole('region', { name: 'Then' });
    expect(within(then).getByText('EMPTY_CART')).toBeDefined();
    expect(within(then).getByText('A cart needs one item.')).toBeDefined();
  });
});

describe('SliceBoard selection', () => {
  it('selects the entity behind a card, in the moment of the slice that declares it', () => {
    const { onSelect } = renderBoard({ slice: 'place-order-slice' });
    fireEvent.click(card('Command: Place order'));
    expect(onSelect).toHaveBeenLastCalledWith(
      key('command', 'place-order'),
      undefined,
      false,
      undefined,
      key('commandSlice', 'place-order-slice'),
    );
    // A neighbor card belongs to the slice that declares it.
    fireEvent.click(card('Read Model: Pending payments'), { shiftKey: true });
    expect(onSelect).toHaveBeenLastCalledWith(
      key('readModel', 'pending-payments'),
      undefined,
      true,
      undefined,
      key('readModelSlice', 'pending-payments-rms'),
    );
    fireEvent.click(screen.getByTestId('slice-canvas'));
    expect(onSelect).toHaveBeenLastCalledWith(undefined, undefined, undefined);
  });

  it('marks the selected cards', () => {
    renderBoard({ slice: 'place-order-slice', selections: [{ type: 'entity', key: key('command', 'place-order') }] });
    expect(card('Command: Place order').getAttribute('aria-pressed')).toBe('true');
    expect(card('Event: Order placed').getAttribute('aria-pressed')).toBe('false');
  });

  it('moves to the slice of a focused entity that is not on the board', async () => {
    const onSelect = vi.fn<OnSelect>();
    const { rerender, lastSlice } = renderBoard({ slice: 'place-order-slice', onSelect });
    const shell = (focus: { key: string; n: number }) => (
      <IssuesProvider issues={EMPTY_ISSUE_INDEX}>
        <SliceBoard model={model} selections={[]} onSelect={onSelect} focus={focus} />
      </IssuesProvider>
    );
    // On the board already: the slice stays.
    rerender(shell({ key: key('readModel', 'order-status'), n: 1 }));
    expect(picker().value).toBe('acme-storefront/place-order-slice');
    rerender(shell({ key: key('event', 'payment.captured'), n: 2 }));
    await waitFor(() => expect(lastSlice()).toBe('acme-storefront/capture-payment-slice'));
    expect(picker().value).toBe('acme-storefront/capture-payment-slice');
  });
});

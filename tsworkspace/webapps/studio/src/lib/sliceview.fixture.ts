// Test fixture for the slice view. Only tests import it, so it stays out of the bundle.
import { buildModel, type Model, type SliceView } from './model';

// A small storefront in the shape of examples/manifests/acme-storefront:
// checkout places an order, an automation captures the payment, and the
// shopper tracks the order status.
export const NS = 'acme-storefront';
export const id = (slug: string, version = '1') => ({ namespace: NS, slug, version });
const schema = (...fields: [string, string][]) => ({
  '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema',
  fields: fields.map(([name, type]) => ({ name, type: { [type]: {} } })),
});

export function storefront(extra: Record<string, unknown>[] = []): Model {
  return buildModel([
    { persona: { id: id('shopper'), title: 'Shopper' } },
    { ui: { id: id('storefront-checkout'), title: 'Checkout' } },
    { ui: { id: id('order-tracking'), title: 'Order tracking' } },
    { swimlane: { id: id('cart'), title: 'Cart', order: 1, state: [{ name: 'item_count', type: { int: {} } }] } },
    { swimlane: { id: id('order'), title: 'Order', order: 2, state: [{ name: 'status', type: { string: {} } }] } },
    {
      command: {
        id: id('add-item-to-cart'),
        title: 'Add item to cart',
        schema: schema(['cart_id', 'uuid'], ['product_id', 'uuid'], ['quantity', 'int']),
      },
    },
    {
      command: {
        id: id('place-order'),
        title: 'Place order',
        schema: schema(['order_id', 'uuid'], ['cart_id', 'uuid'], ['buyer_id', 'uuid']),
      },
    },
    {
      command: {
        id: id('capture-payment'),
        title: 'Capture payment',
        schema: schema(['order_id', 'uuid'], ['amount', 'int']),
      },
    },
    {
      event: {
        id: id('cart.item-added'),
        title: 'Item added to cart',
        swimlane: { id: id('cart') },
        schema: schema(['cart_id', 'uuid'], ['product_id', 'uuid'], ['quantity', 'int']),
      },
    },
    {
      event: {
        id: id('order.placed'),
        title: 'Order placed',
        swimlane: { id: id('order') },
        schema: schema(['order_id', 'uuid'], ['cart_id', 'uuid'], ['buyer_id', 'uuid']),
      },
    },
    {
      event: {
        id: id('payment.captured'),
        title: 'Payment captured',
        swimlane: { id: id('order') },
        schema: schema(['order_id', 'uuid'], ['amount', 'int']),
      },
    },
    {
      readModel: {
        id: id('cart-summary'),
        title: 'Cart summary',
        schema: schema(['cart_id', 'uuid'], ['item_count', 'int']),
      },
    },
    {
      // Missing cart_id on purpose: the field stops at this projection.
      readModel: {
        id: id('pending-payments'),
        title: 'Pending payments',
        schema: schema(['order_id', 'uuid'], ['buyer_id', 'uuid']),
      },
    },
    {
      readModel: {
        id: id('order-status'),
        title: 'Order status',
        schema: schema(['order_id', 'uuid'], ['status', 'string']),
      },
    },
    { processor: { id: id('payment-capture-worker'), title: 'Payment capture worker' } },
    {
      commandSlice: {
        id: id('add-item-to-cart-slice'),
        title: 'Add item to cart',
        persona: { persona: { id: id('shopper') } },
        ui: { ui: { id: id('storefront-checkout') } },
        command: { command: { id: id('add-item-to-cart') } },
        emittedEvents: [{ event: { id: id('cart.item-added') } }],
      },
    },
    {
      readModelSlice: {
        id: id('cart-summary-rms'),
        title: 'Cart summary',
        readModel: { readModel: { id: id('cart-summary') } },
        sourceEvents: [{ event: { id: id('cart.item-added') } }],
        scenarios: [
          {
            id: 'first-item',
            title: 'First item',
            when: [{ event: { id: id('cart.item-added') } }],
            // oxlint-disable-next-line unicorn/no-thenable -- `then` is the ReadModelScenario proto field
            then: { readModel: { id: id('cart-summary') }, payload: { item_count: 1 } },
          },
        ],
      },
    },
    {
      commandSlice: {
        id: id('place-order-slice'),
        title: 'Place order',
        persona: { persona: { id: id('shopper') } },
        ui: { ui: { id: id('storefront-checkout') } },
        command: { command: { id: id('place-order') } },
        emittedEvents: [{ event: { id: id('order.placed') } }],
        scenarios: [
          {
            id: 'placed',
            title: 'Order placed',
            given: [{ event: { id: id('cart.item-added') } }],
            when: { command: { id: id('place-order') }, payload: { cart_id: 'c-1' } },
            emit: { events: [{ event: { id: id('order.placed') } }] },
          },
          {
            id: 'empty-cart',
            title: 'Empty cart',
            when: { command: { id: id('place-order') } },
            reject: { reasonCode: 'EMPTY_CART', doc: 'A cart needs one item.' },
          },
        ],
      },
    },
    {
      readModelSlice: {
        id: id('pending-payments-rms'),
        title: 'Pending payments',
        readModel: { readModel: { id: id('pending-payments') } },
        sourceEvents: [{ event: { id: id('order.placed') } }],
      },
    },
    {
      readModelSlice: {
        id: id('order-status-placed-rms'),
        title: 'Order status: placed',
        readModel: { readModel: { id: id('order-status') } },
        sourceEvents: [{ event: { id: id('order.placed') } }],
      },
    },
    {
      automationSlice: {
        id: id('capture-payment-auto'),
        title: 'Capture payment',
        processor: { processor: { id: id('payment-capture-worker') } },
        emittedCommand: { command: { id: id('capture-payment') } },
        sourceReadModels: [{ readModel: { id: id('pending-payments') } }],
        scenarios: [
          {
            id: 'ok',
            title: 'Payment captured',
            given: [{ readModel: { id: id('pending-payments') } }],
            whenDoc: 'The payment provider confirms the capture.',
            // oxlint-disable-next-line unicorn/no-thenable -- `then` is the AutomationScenario proto field
            then: { command: { id: id('capture-payment') } },
          },
        ],
      },
    },
    {
      commandSlice: {
        id: id('capture-payment-slice'),
        title: 'Capture payment',
        command: { command: { id: id('capture-payment') } },
        emittedEvents: [{ event: { id: id('payment.captured') } }],
      },
    },
    {
      readModelSlice: {
        id: id('order-status-paid-rms'),
        title: 'Order status: paid',
        readModel: { readModel: { id: id('order-status') } },
        sourceEvents: [{ event: { id: id('payment.captured') } }],
      },
    },
    {
      uiSlice: {
        id: id('order-tracking-ui-slice'),
        title: 'Shopper tracks the order',
        persona: { persona: { id: id('shopper') } },
        ui: { ui: { id: id('order-tracking') } },
        sourceReadModels: [{ readModel: { id: id('order-status') } }],
      },
    },
    {
      storyboard: {
        id: id('checkout-flow'),
        title: 'Checkout',
        slices: [
          { id: id('add-item-to-cart-slice') },
          { id: id('cart-summary-rms') },
          { id: id('place-order-slice') },
          { id: id('pending-payments-rms') },
          { id: id('order-status-placed-rms') },
        ],
      },
    },
    {
      storyboard: {
        id: id('fulfillment-flow'),
        title: 'Fulfillment',
        slices: [
          { id: id('capture-payment-auto') },
          { id: id('capture-payment-slice') },
          { id: id('order-status-paid-rms') },
          { id: id('order-tracking-ui-slice') },
        ],
      },
    },
    {
      eventModel: {
        id: id('storefront'),
        title: 'Storefront',
        members: [
          { kind: 'ENTITY_KIND_STORYBOARD', id: id('checkout-flow') },
          { kind: 'ENTITY_KIND_STORYBOARD', id: id('fulfillment-flow') },
        ],
      },
    },
    ...extra,
  ] as never);
}

export function sliceNamed(model: Model, slug: string): SliceView {
  const slice = model.slices.find((s) => s.entity.id.slug === slug);
  if (!slice) throw new Error(`no slice ${slug}`);
  return slice;
}

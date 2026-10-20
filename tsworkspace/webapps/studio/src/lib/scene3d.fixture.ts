// A small storefront model in wire shape for the 3D layout tests: a cart
// stream, an order stream and a fulfillment stream that shares the order's
// stream id, two storyboards, and automations between them. Code
// references are optional, so a test can check the code wall both ways.

const NS = 'acme-storefront';
const id = (slug: string) => ({ namespace: NS, slug, version: '1' });
const ref = (slug: string) => ({ id: id(slug) });

const CODE_REF = 'type.googleapis.com/trogonatlas.annotation.v1alpha1.CodeRefAnnotation';
const code = (repo: string, path: string, symbol?: string) => [{ '@type': CODE_REF, repo, path, symbol }];

export function storefrontEntities({ codeRefs = false }: { codeRefs?: boolean } = {}): Record<string, unknown>[] {
  const meta = (repo: string, path: string, symbol?: string) =>
    codeRefs ? { metadata: code(repo, path, symbol) } : {};
  return [
    { persona: { id: id('shopper'), title: 'Shopper' } },
    { ui: { id: id('checkout'), title: 'Checkout' } },
    { ui: { id: id('order-status-page'), title: 'Order status page' } },
    { swimlane: { id: id('cart'), title: 'Cart', order: 1, streamId: 'cart-{cart_id}' } },
    { swimlane: { id: id('order'), title: 'Order', order: 2, streamId: 'order-{order_id}' } },
    { swimlane: { id: id('fulfillment'), title: 'Fulfillment', order: 3, streamId: 'order-{order_id}' } },
    {
      command: {
        id: id('add-item-to-cart'),
        title: 'Add item to cart',
        ...meta('storefront', 'apps/storefront/lib/storefront/cart/add_item.ex', 'Storefront.Cart.AddItem'),
      },
    },
    {
      command: {
        id: id('place-order'),
        title: 'Place order',
        ...meta('storefront', 'apps/storefront/lib/storefront/orders/place_order.ex', 'Storefront.Orders.PlaceOrder'),
      },
    },
    { command: { id: id('capture-payment'), title: 'Capture payment' } },
    { command: { id: id('dispatch-order'), title: 'Dispatch order' } },
    {
      event: {
        id: id('cart.item-added'),
        title: 'Item added to cart',
        swimlane: ref('cart'),
        ...meta('storefront', 'apps/storefront/lib/storefront/cart/events.ex', 'Storefront.Cart.ItemAdded'),
      },
    },
    {
      event: {
        id: id('order.placed'),
        title: 'Order placed',
        swimlane: ref('order'),
        ...meta('storefront', 'apps/storefront/lib/storefront/orders/events.ex', 'Storefront.Orders.Placed'),
      },
    },
    {
      event: {
        id: id('payment.captured'),
        title: 'Payment captured',
        swimlane: ref('order'),
        ...meta('payments', 'services/capture/events.go', 'capture.Captured'),
      },
    },
    { event: { id: id('order.dispatched'), title: 'Order dispatched', swimlane: ref('fulfillment') } },
    { readModel: { id: id('cart-summary'), title: 'Cart summary', sourceEvents: [ref('cart.item-added')] } },
    { readModel: { id: id('pending-payments'), title: 'Pending payments', sourceEvents: [ref('order.placed')] } },
    {
      readModel: {
        id: id('order-status'),
        title: 'Order status',
        sourceEvents: [ref('payment.captured'), ref('order.dispatched')],
        ...meta('storefront', 'apps/storefront_web/live/order_status_live.ex', 'StorefrontWeb.OrderStatusLive'),
      },
    },
    {
      processor: {
        id: id('payment-capture-worker'),
        title: 'Payment capture worker',
        ...meta('payments', 'services/capture/worker.go', 'capture.Worker'),
      },
    },
    { processor: { id: id('dispatch-worker'), title: 'Dispatch worker' } },
    {
      commandSlice: {
        id: id('add-item-to-cart-slice'),
        title: 'Add item to cart',
        persona: { persona: ref('shopper') },
        ui: { ui: ref('checkout') },
        command: { command: ref('add-item-to-cart') },
        emittedEvents: [{ event: ref('cart.item-added') }],
      },
    },
    {
      readModelSlice: {
        id: id('cart-summary-rms'),
        title: 'Cart summary',
        readModel: { readModel: ref('cart-summary') },
        sourceEvents: [{ event: ref('cart.item-added') }],
      },
    },
    {
      commandSlice: {
        id: id('place-order-slice'),
        title: 'Place order',
        persona: { persona: ref('shopper') },
        ui: { ui: ref('checkout') },
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
        processor: { processor: ref('payment-capture-worker') },
        sourceReadModels: [{ readModel: ref('pending-payments') }],
        emittedCommand: { command: ref('capture-payment') },
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
      readModelSlice: {
        id: id('order-status-paid-rms'),
        title: 'Order status: paid',
        readModel: { readModel: ref('order-status') },
        sourceEvents: [{ event: ref('payment.captured') }],
      },
    },
    {
      automationSlice: {
        id: id('dispatch-auto'),
        title: 'Dispatch',
        processor: { processor: ref('dispatch-worker') },
        sourceReadModels: [{ readModel: ref('order-status') }],
        emittedCommand: { command: ref('dispatch-order') },
      },
    },
    {
      commandSlice: {
        id: id('dispatch-order-slice'),
        title: 'Dispatch order',
        command: { command: ref('dispatch-order') },
        emittedEvents: [{ event: ref('order.dispatched') }],
      },
    },
    {
      readModelSlice: {
        id: id('order-status-dispatched-rms'),
        title: 'Order status: dispatched',
        readModel: { readModel: ref('order-status') },
        sourceEvents: [{ event: ref('order.dispatched') }],
      },
    },
    {
      uiSlice: {
        id: id('order-status-ui'),
        title: 'Shopper sees the order status',
        persona: { persona: ref('shopper') },
        ui: { ui: ref('order-status-page') },
        sourceReadModels: [{ readModel: ref('order-status') }],
      },
    },
    {
      storyboard: {
        id: id('checkout-flow'),
        title: 'Checkout',
        slices: [
          ref('add-item-to-cart-slice'),
          ref('cart-summary-rms'),
          ref('place-order-slice'),
          ref('pending-payments-rms'),
        ],
      },
    },
    {
      storyboard: {
        id: id('fulfillment-flow'),
        title: 'Fulfillment',
        slices: [
          ref('capture-payment-auto'),
          ref('capture-payment-slice'),
          ref('order-status-paid-rms'),
          ref('dispatch-auto'),
          ref('dispatch-order-slice'),
          ref('order-status-dispatched-rms'),
          ref('order-status-ui'),
        ],
      },
    },
    {
      eventModel: {
        id: id('acme-storefront-model'),
        title: 'Acme storefront',
        members: [
          { kind: 'ENTITY_KIND_STORYBOARD', id: id('checkout-flow') },
          { kind: 'ENTITY_KIND_STORYBOARD', id: id('fulfillment-flow') },
        ],
      },
    },
  ];
}

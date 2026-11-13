import { buildModel, type EntityId } from './model';

const id = (slug: string): EntityId => ({ namespace: 'preview-storefront', slug, version: '1' });
const ref = (slug: string) => ({ id: id(slug) });

export function buildFeaturePreviewModel() {
  const entities: Parameters<typeof buildModel>[0] = [
    { swimlane: { id: id('orders'), title: 'Orders', order: 1, streamId: 'order-{order_id}' } },
    { swimlane: { id: id('payments'), title: 'Payments', order: 2, streamId: 'payment-{payment_id}' } },
    { command: { id: id('place-order'), title: 'Place order' } },
    { command: { id: id('capture-payment'), title: 'Capture payment' } },
    { event: { id: id('order.placed'), title: 'Order placed', swimlane: ref('orders') } },
    { event: { id: id('payment.started'), title: 'Payment started', swimlane: ref('payments') } },
    { event: { id: id('payment.captured'), title: 'Payment captured', swimlane: ref('payments') } },
    { readModel: { id: id('pending-payments'), title: 'Pending payments', sourceEvents: [ref('order.placed')] } },
    { readModel: { id: id('order-status'), title: 'Order status', sourceEvents: [ref('payment.captured')] } },
    { processor: { id: id('payment-worker'), title: 'Payment worker' } },
    {
      commandSlice: {
        id: id('place-order-slice'),
        title: 'Place order',
        command: { command: ref('place-order') },
        emittedEvents: [{ event: ref('order.placed') }],
      },
    },
    {
      readModelSlice: {
        id: id('pending-payments-slice'),
        title: 'Pending payments',
        readModel: { readModel: ref('pending-payments') },
        sourceEvents: [{ event: ref('order.placed') }],
      },
    },
    {
      automationSlice: {
        id: id('capture-payment-automation'),
        title: 'Capture payment automatically',
        processor: { processor: ref('payment-worker') },
        sourceReadModels: [{ readModel: ref('pending-payments') }],
        emittedCommand: { command: ref('capture-payment') },
      },
    },
    {
      commandSlice: {
        id: id('capture-payment-slice'),
        title: 'Capture payment',
        command: { command: ref('capture-payment') },
        emittedEvents: [{ event: ref('payment.started') }, { event: ref('payment.captured') }],
      },
    },
    {
      readModelSlice: {
        id: id('order-status-slice'),
        title: 'Show order status',
        readModel: { readModel: ref('order-status') },
        sourceEvents: [{ event: ref('payment.captured') }],
      },
    },
    {
      storyboard: {
        id: id('checkout'),
        title: 'Example checkout',
        slices: [
          ref('place-order-slice'),
          ref('pending-payments-slice'),
          ref('capture-payment-automation'),
          ref('capture-payment-slice'),
          ref('order-status-slice'),
        ],
      },
    },
  ];
  return buildModel(entities);
}

/** A fictional order lifecycle for the State machine preview: forks, a final state and a return. */
export function buildStateMachinePreviewModel() {
  const lane = ref('order-lifecycle');
  const event = (slug: string, title: string) => ({ event: { id: id(slug), title, swimlane: lane } });
  const next = (slug: string, doc: string) => ({ event: ref(slug), doc });
  const entities: Parameters<typeof buildModel>[0] = [
    {
      swimlane: {
        id: id('order-lifecycle'),
        title: 'Order',
        order: 1,
        streamId: 'order-{order_id}',
        transitions: [
          {
            after: ref('order.placed'),
            next: [
              next('payment.authorized', 'The card authorizes before the hold expires.'),
              next('order.cancelled', 'The buyer cancels, or the hold expires first.'),
            ],
          },
          {
            after: ref('payment.authorized'),
            next: [
              next('order.shipped', 'The warehouse ships before the authorization lapses.'),
              next('payment.voided', 'The authorization lapses before shipping.'),
            ],
          },
          { after: ref('payment.voided'), next: [next('payment.authorized', 'The card authorizes again.')] },
          { after: ref('order.shipped'), next: [next('order.delivered', 'The carrier confirms delivery.')] },
          { after: ref('order.delivered') },
          { after: ref('order.cancelled') },
        ],
      },
    },
    event('order.placed', 'Order placed'),
    event('payment.authorized', 'Payment authorized'),
    event('payment.voided', 'Payment voided'),
    event('order.shipped', 'Order shipped'),
    event('order.delivered', 'Order delivered'),
    event('order.cancelled', 'Order cancelled'),
  ];
  return buildModel(entities);
}

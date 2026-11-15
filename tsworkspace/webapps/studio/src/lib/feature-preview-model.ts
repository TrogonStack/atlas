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

import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildModel, wireKind, type Entity, type Model } from '@/lib/model';
import { isApiSpecification, resolveContractType } from '@/lib/specification';
import { SpecificationDetails } from './SpecificationDetails';

const id = (slug: string, version = '1') => ({ namespace: 'orders', slug, version });
const memberId = (value: string) => ({ value });
const digest = { sha256: 'a'.repeat(64) };
function fixture() {
  return buildModel([
    { readModel: { id: id('request'), title: 'Request data' } },
    { readModel: { id: id('receipt', '2'), title: 'Receipt data' } },
    { readModel: { id: id('receipt', '3'), title: 'New receipt data' } },
    { component: { id: id('public'), title: 'Public gateway' } },
    { component: { id: id('partner'), title: 'Partner gateway' } },
    { component: { id: id('checkout'), title: 'Checkout' } },
    {
      system: { uid: 'contract-owner' },
      specification: {
        id: id('order-api'),
        title: 'Order API',
        artifacts: [
          {
            memberId: memberId('artifact'),
            name: memberId('Contract source'),
            path: memberId('contracts/order.json'),
            format: { known: 'KNOWN_JSON_SCHEMA' },
            mediaType: memberId('application/json'),
            content: '{\r\n  "type": "object"\r\n}\r\n',
            digest,
          },
        ],
        schemas: [
          {
            memberId: memberId('shape'),
            name: memberId('Order shape'),
            definition: {
              connectionId: memberId('definition'),
              artifact: { artifactId: memberId('artifact'), expectedDigest: digest },
              selector: { syntax: 'json-pointer', value: '/$defs/Order' },
            },
          },
        ],
        messages: [
          {
            memberId: memberId('request'),
            name: memberId('Order request'),
            schemas: [
              { connectionId: memberId('request-shape'), role: 'ROLE_PAYLOAD', localSchema: memberId('shape') },
            ],
            mapping: { connectionId: memberId('request-map'), readModel: { id: id('request') } },
          },
          {
            memberId: memberId('response'),
            name: memberId('Order receipt'),
            mapping: { connectionId: memberId('response-map'), readModel: { id: id('receipt', '2') } },
          },
          { memberId: memberId('unmapped'), name: memberId('Unmapped message') },
        ],
        operations: [
          {
            memberId: memberId('submit'),
            name: memberId('Submit'),
            binding: { http: { method: 'POST', path: '/orders', responseStatusCodes: [201] } },
            messages: [
              { connectionId: memberId('request-occurrence'), messageId: memberId('request'), role: 'ROLE_REQUEST' },
              { connectionId: memberId('response-occurrence'), messageId: memberId('response'), role: 'ROLE_RESPONSE' },
              {
                connectionId: memberId('notification-occurrence'),
                messageId: memberId('response'),
                role: 'ROLE_NOTIFICATION',
              },
            ],
          },
          {
            memberId: memberId('lookup'),
            name: memberId('Lookup'),
            messages: [
              { connectionId: memberId('another-response'), messageId: memberId('response'), role: 'ROLE_RESPONSE' },
            ],
          },
        ],
        exposures: [
          {
            connectionId: memberId('public-exposure'),
            provider: { component: { id: id('public') } },
            operationId: memberId('submit'),
          },
          {
            connectionId: memberId('partner-exposure'),
            provider: { component: { id: id('partner') } },
            operationId: memberId('submit'),
          },
        ],
        usages: [
          {
            connectionId: memberId('checkout-usage'),
            caller: { component: { id: id('checkout') } },
            operationId: memberId('submit'),
            exposureId: memberId('public-exposure'),
          },
          {
            connectionId: memberId('unspecified-usage'),
            caller: { component: { id: id('checkout') } },
            operationId: memberId('submit'),
          },
        ],
      },
    },
  ]);
}
function specification(model: Model): Entity {
  const entity = model.byKind.get('specification')?.[0];
  if (!entity) throw new Error('Specification fixture missing');
  return entity;
}
function typeSelection() {
  return {
    schema: {
      specification: { id: id('order-api') },
      ownerUid: 'contract-owner',
      kind: 'CONTRACT_MEMBER_KIND_SCHEMA',
      memberId: memberId('shape'),
    },
    expectedArtifactId: memberId('artifact'),
    expectedDigest: digest,
    expectedSelector: { syntax: 'json-pointer', value: '/$defs/Order' },
  };
}
afterEach(cleanup);

describe('Specification review', () => {
  it('normalizes the native kind and derives API classification without adding behavior', () => {
    const model = fixture();
    const entity = specification(model);
    expect(wireKind('ENTITY_KIND_SPECIFICATION')).toBe('specification');
    expect(model.slices).toHaveLength(0);
    expect(isApiSpecification(entity)).toBe(true);
    expect(isApiSpecification({ ...entity, raw: { ...entity.raw, exposures: [] } })).toBe(false);
    expect(isApiSpecification({ ...entity, raw: { ...entity.raw, operations: [] } })).toBe(false);
    expect(
      isApiSpecification({ ...entity, raw: { ...entity.raw, exposures: [{ operationId: memberId('unknown') }] } }),
    ).toBe(false);
    const restrictedProjection = buildModel([{ system: { uid: entity.uid }, specification: entity.raw }]);
    expect(isApiSpecification(specification(restrictedProjection))).toBe(true);
  });
  it('renders retained content, unverified selectors, separate roles and exact mapped versions without edit controls', () => {
    const model = fixture();
    const onSelect = vi.fn();
    const { container } = render(
      <SpecificationDetails entity={specification(model)} model={model} onSelect={onSelect} />,
    );
    expect(screen.getByRole('heading', { name: 'API specification' })).toBeDefined();
    expect(container.querySelector('pre')?.textContent).toBe('{\r\n  "type": "object"\r\n}\r\n');
    expect(screen.getByText('json-pointer: /$defs/Order (unverified)')).toBeDefined();
    expect(screen.getByText('REQUEST')).toBeDefined();
    expect(screen.getAllByText('RESPONSE')).toHaveLength(2);
    expect(screen.getByText('NOTIFICATION')).toBeDefined();
    for (const identity of ['request-occurrence', 'response-occurrence', 'notification-occurrence', 'another-response'])
      expect(screen.getByText(identity)).toBeDefined();
    expect(screen.getByText('ReadModel mapping undeclared')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'orders/receipt@2' }));
    expect(onSelect).toHaveBeenCalledWith('readModel:orders/receipt@2');
    expect(screen.queryByRole('button', { name: 'orders/receipt@3' })).toBeNull();
    expect(container.querySelectorAll('input, textarea, select, [contenteditable="true"]')).toHaveLength(0);
  });
  it('shows only the selected provider per usage and keeps provider absence explicit', () => {
    const model = fixture();
    render(<SpecificationDetails entity={specification(model)} model={model} onSelect={vi.fn()} />);
    const usage = screen.getByText('checkout-usage').closest('dl');
    if (!usage) throw new Error('Usage declaration missing');
    expect(within(usage).getByRole('button', { name: 'orders/public@1' })).toBeDefined();
    expect(within(usage).queryByRole('button', { name: 'orders/partner@1' })).toBeNull();
    const unspecified = screen.getByText('unspecified-usage').closest('dl');
    if (!unspecified) throw new Error('Unspecified usage declaration missing');
    expect(within(unspecified).getByText('Provider undeclared')).toBeDefined();
    expect(within(unspecified).queryByRole('button', { name: /orders\/(public|partner)@1/ })).toBeNull();
  });
  it('renders native, library and external pinned schema bindings independently of transport declarations', () => {
    const model = fixture();
    const original = specification(model);
    const selected = {
      ...original,
      raw: {
        ...original.raw,
        messages: [
          {
            memberId: memberId('bindings'),
            name: memberId('Typed message'),
            schemas: [
              {
                connectionId: memberId('native-binding'),
                role: 'ROLE_HEADERS',
                schema: { id: id('native-shape', '2') },
              },
              {
                connectionId: memberId('library-binding'),
                role: 'ROLE_PAYLOAD',
                libraryType: { library: { id: id('types') }, message: memberId('orders.Receipt') },
              },
              { connectionId: memberId('external-binding'), role: 'ROLE_PAYLOAD', contractType: typeSelection() },
            ],
          },
        ],
        operations: [
          {
            memberId: memberId('transport-operation'),
            name: memberId('Transport operation'),
            binding: { channel: { connectionId: memberId('channel-binding'), channel: { id: id('transport') } } },
            messages: [],
          },
        ],
      },
    };
    const onSelect = vi.fn();
    render(<SpecificationDetails entity={selected} model={model} onSelect={onSelect} />);
    expect(screen.getByText('orders/native-shape@2 (not loaded)')).toBeDefined();
    expect(screen.getByText('orders/types@1 (not loaded)')).toBeDefined();
    expect(screen.getByText('orders.Receipt', { exact: false })).toBeDefined();
    expect(screen.getByText('channel-binding')).toBeDefined();
    expect(screen.getByText('orders/transport@1 (not loaded)')).toBeDefined();
    const external = screen.getByText('external-binding').closest('dl');
    if (!external) throw new Error('External binding missing');
    expect(within(external).getByText('Order shape')).toBeDefined();
    expect(within(external).getByText('json-pointer: /$defs/Order (unverified)')).toBeDefined();
    fireEvent.click(within(external).getByRole('button', { name: 'orders/order-api@1' }));
    expect(onSelect).toHaveBeenCalledWith('specification:orders/order-api@1');
  });
  it('resolves external schema pins by owner incarnation, member identity, digest and selector presence', () => {
    const model = fixture();
    const selection = typeSelection();
    expect(resolveContractType(model, selection)?.name).toEqual(memberId('Order shape'));
    for (const stale of [
      { ...selection, schema: { ...selection.schema, ownerUid: 'retired-owner' } },
      { ...selection, schema: { ...selection.schema, ownerUid: '' } },
      { ...selection, schema: { ...selection.schema, expectedDigest: digest } },
      { ...selection, schema: { ...selection.schema, kind: 'CONTRACT_MEMBER_KIND_MESSAGE' } },
      { ...selection, schema: { ...selection.schema, memberId: memberId('other') } },
      { ...selection, expectedArtifactId: memberId('replacement') },
      { ...selection, expectedDigest: { sha256: 'b'.repeat(64) } },
      { ...selection, expectedSelector: undefined },
      { ...selection, expectedSelector: { syntax: 'json-pointer', value: '/$defs/Other' } },
    ])
      expect(resolveContractType(model, stale)).toBeUndefined();
  });
});

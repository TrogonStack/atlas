import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildModel } from '@/lib/model';
import * as api from '@/lib/api';
import { DataProductInspector, offeringLabel } from './DataProductInspector';

const state = (name: string) => `DATA_PRODUCT_SELECTION_STATE_${name}`;
const portId = (value: string) => ({ value });
const product = {
  id: { namespace: 'shop', slug: 'offering', version: '2' },
  title: 'Offering',
  inputs: [
    {
      portId: portId('input-a'),
      name: { value: 'Local fact' },
      data: { connectionId: portId('input-assoc'), event: { id: { namespace: 'shop', slug: 'fact', version: '3' } } },
    },
  ],
  outputs: [
    {
      portId: portId('output-a'),
      name: { value: 'Receipt' },
      data: {
        connectionId: portId('output-assoc'),
        readModel: { id: { namespace: 'shop', slug: 'receipt', version: '4' } },
      },
      contract: {
        connectionId: portId('type-assoc'),
        contractType: {
          expectedDigest: { sha256: 'a'.repeat(64) },
          schema: { ownerUid: 'spec-uid', memberId: portId('schema-id') },
        },
      },
      derivedFrom: { inputs: [{ connectionId: portId('lineage-assoc'), inputPortId: portId('input-a') }] },
      access: ['access-a', 'access-b'].map((id) => ({
        connectionId: portId(id),
        operation: {
          operation: { ownerUid: 'spec-uid', memberId: portId('operation-id') },
          response: { ownerUid: 'spec-uid', id: portId('response-id') },
        },
      })),
    },
    { portId: portId('output-b'), name: { value: 'Empty' }, derivedFrom: {} },
    { portId: portId('output-c'), name: { value: 'Unknown' } },
  ],
  resourceAssociations: [
    {
      connectionId: portId('assoc-a'),
      direction: 'DATA_PORT_DIRECTION_OUTPUT',
      event: { id: { namespace: 'shop', slug: 'fact', version: '3' } },
      doc: 'authored',
    },
    {
      connectionId: portId('assoc-b'),
      direction: 'DATA_PORT_DIRECTION_OUTPUT',
      event: { id: { namespace: 'shop', slug: 'fact', version: '3' } },
    },
  ],
};
const inspection: api.DataProductInspection = {
  product,
  offering: 'DATA_PRODUCT_OFFERING_STATE_DRAFT',
  etag: 'etag-checked',
  scope: { revision: 'checked-revision', branch: 'review', authorizedNamespaces: ['shop'] },
  ports: [
    { port: { portId: portId('input-a'), direction: 'DATA_PORT_DIRECTION_INPUT' }, data: state('RESOLVED') },
    ...['output-a', 'output-b', 'output-c'].map((id) => ({
      port: { portId: portId(id), direction: 'DATA_PORT_DIRECTION_OUTPUT' },
      data: state(id === 'output-a' ? 'RESOLVED' : 'UNDECLARED'),
      contract: state('OPAQUE'),
      lineage: state('DECLARED'),
      access: product.outputs[0].access?.map((declaration) => ({
        declaration,
        state: state('RESOLVED'),
        provider: state('UNDECLARED'),
      })),
    })),
  ],
  emissions: [{ emission: { ownerUid: 'slice-uid', id: portId('emission-id') } }],
  producing: [{ path: [{ id: portId('producing-edge') }] }],
};
function entity() {
  const entity = buildModel([{ dataProduct: product }]).byKind.get('dataProduct')?.[0];
  if (!entity) throw new Error('fixture missing');
  return entity;
}
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.history.replaceState({}, '', '/');
});
describe('DataProduct backend inspection', () => {
  it('preserves IDs, repeated access, optional declarations and checked evidence', async () => {
    window.history.replaceState({}, '', '/?branch=review');
    const fetch = vi.spyOn(api, 'inspectDataProduct').mockResolvedValue(inspection);
    render(<DataProductInspector entity={entity()} />);
    await screen.findByText('Draft offering');
    expect(fetch).toHaveBeenCalledWith(product.id, expect.objectContaining({ branch: 'review' }));
    for (const text of ['input-a', 'input-assoc', 'output-a', 'output-assoc', 'access-a', 'access-b'])
      expect(screen.getByText(text)).toBeDefined();
    expect(screen.getAllByText('Undeclared (contract-level access)')).toHaveLength(2);
    expect(screen.getByText(/Explicit empty declaration/)).toBeDefined();
    expect(screen.getByText('Unknown (absent declaration)')).toBeDefined();
    expect(screen.getByText(/checked-revision/)).toBeDefined();
    expect(screen.getByText(/emission-id/)).toBeDefined();
    expect(screen.getByText(/producing-edge/)).toBeDefined();
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.getByText('assoc-a')).toBeDefined();
    expect(screen.getByText('assoc-b')).toBeDefined();
    expect(screen.getByText(/Descriptive associations/)).toBeDefined();
    expect(screen.getByText('authored')).toBeDefined();
  });
  it('does not crash when the product record has no associations', async () => {
    vi.spyOn(api, 'inspectDataProduct').mockResolvedValue({ ...inspection, product: {} });
    const bare = entity();
    bare.raw = {};
    render(<DataProductInspector entity={bare} />);
    expect(screen.getByText('None declared')).toBeDefined();
  });
  it('keeps backend failures unknown and does not infer mapped data from raw CRUD', async () => {
    vi.spyOn(api, 'inspectDataProduct').mockRejectedValue(new Error('permission denied'));
    render(<DataProductInspector entity={entity()} />);
    await screen.findByRole('alert');
    expect(screen.getByText('Unknown: backend inspection unavailable')).toBeDefined();
    expect(screen.getAllByText('Unchecked selection')).toHaveLength(2);
    expect(screen.queryByText('Confirmed mapped offering')).toBeNull();
  });
  it('does not reuse live evidence for historical declarations', async () => {
    window.history.replaceState({}, '', '/?asOf=11111111-1111-4111-8111-111111111111');
    const fetch = vi.spyOn(api, 'inspectDataProduct');
    render(<DataProductInspector entity={entity()} />);
    await waitFor(() => expect(screen.getByText(/Historical inspection unavailable/)).toBeDefined());
    expect(fetch).not.toHaveBeenCalled();
  });
  it('distinguishes stale/inaccessible/incomplete states from undeclared draft and mapped outputs', () => {
    expect(offeringLabel(inspection)).toBe('Draft offering');
    expect(
      offeringLabel({ ...inspection, incomplete: true, offering: 'DATA_PRODUCT_OFFERING_STATE_UNKNOWN' }),
    ).toContain('Unknown');
    expect(offeringLabel({ ...inspection, offering: 'DATA_PRODUCT_OFFERING_STATE_UNRESOLVED' })).toBe('Unresolved');
    expect(offeringLabel({ ...inspection, offering: 'DATA_PRODUCT_OFFERING_STATE_MAPPED' })).toBe(
      'Confirmed mapped offering',
    );
    expect(offeringLabel({ ...inspection, offering: undefined })).toContain('Unknown');
    expect(offeringLabel({ product: { outputs: [{ data: {} }] } })).toContain('Unknown');
  });
});

it('uses retained historical context before the URL catches up', () => {
  window.history.replaceState({}, '', '/?branch=review');
  const fetch = vi.spyOn(api, 'inspectDataProduct');
  render(<DataProductInspector entity={entity()} asOf="11111111-1111-4111-8111-111111111111" />);
  expect(screen.getByText(/Historical inspection unavailable/)).toBeDefined();
  expect(fetch).not.toHaveBeenCalled();
});

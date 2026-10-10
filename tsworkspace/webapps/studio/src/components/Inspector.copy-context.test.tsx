// BUG proof: Inspector's drawer Timeline is moment-aware (neighborsOfMoment
// when momentKey is set), but Copy context always calls entityContext which
// uses neighborsOf, so the clipboard describes the type-level graph, not
// the occurrence the user is inspecting.
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/useEntityHomes', () => ({
  useEntityHomes: () => ({ homes: new Map(), error: undefined, retry: vi.fn() }),
}));

import type { WireEntityEnvelope } from '@/lib/api';
import { buildIssueIndex } from '@/lib/issues';
import { buildModel } from '@/lib/model';
import { Inspector } from './Inspector';

const mkId = (ns: string, slug: string) => ({ namespace: ns, slug, version: '1' });

function twoSliceFixture(): WireEntityEnvelope[] {
  const ns = 'orders';
  return [
    { ui: { id: mkId(ns, 'cart'), title: 'Cart' } },
    { ui: { id: mkId(ns, 'admin'), title: 'Admin' } },
    { command: { id: mkId(ns, 'place'), title: 'Place order' } },
    { event: { id: mkId(ns, 'placed'), title: 'Order placed' } },
    {
      commandSlice: {
        id: mkId(ns, 's-cart'),
        title: 'place from cart',
        ui: { ui: { id: mkId(ns, 'cart') } },
        command: { command: { id: mkId(ns, 'place') } },
        emittedEvents: [{ event: { id: mkId(ns, 'placed') } }],
      },
    },
    {
      commandSlice: {
        id: mkId(ns, 's-admin'),
        title: 'place from admin',
        ui: { ui: { id: mkId(ns, 'admin') } },
        command: { command: { id: mkId(ns, 'place') } },
        emittedEvents: [{ event: { id: mkId(ns, 'placed') } }],
      },
    },
  ] as never;
}

describe('Inspector copy context vs moment timeline', () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    window.history.replaceState({}, '', '/');
  });

  it('copies moment-scoped neighbors when momentKey is set, matching the Timeline', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });

    const model = buildModel(twoSliceFixture());
    const cmd = model.entities.find((e) => e.kind === 'command');
    const cartSlice = model.slices.find((s) => s.entity.id.slug === 's-cart');
    expect(cmd).toBeDefined();
    expect(cartSlice).toBeDefined();

    render(
      <Inspector model={model} entity={cmd!} momentKey={cartSlice!.entity.key} onClose={vi.fn()} onSelect={vi.fn()} />,
    );

    // Drawer timeline for this moment only shows Cart, not Admin.
    expect(screen.getByText('Timeline: this moment')).toBeDefined();
    expect(screen.getByText('Cart')).toBeDefined();
    expect(screen.queryByText('Admin')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: /copy context/i }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalled();
    });
    const copied = String(writeText.mock.calls[0]?.[0] ?? '');
    expect(copied).toContain('Cart');
    // Must not leak the other occurrence's neighbor into the clipboard.
    expect(copied).not.toContain('Admin');
  });

  it('copies a ready-to-paste repair request directly from a validation finding', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    window.history.replaceState({}, '', '/em/orders/order-service?branch=repair/context&token=do-not-copy');
    const model = buildModel(twoSliceFixture());
    const command = model.entities.find((entity) => entity.kind === 'command');
    if (!command) throw new Error('Missing command fixture');
    const issues = buildIssueIndex([
      {
        severity: 'SEVERITY_ERROR',
        code: 'COMMAND_MISSING_FIELD',
        message: 'The order identifier needs a source.',
        subjectField: 'schema.fields.order_id',
        subject: { kind: 'ENTITY_KIND_COMMAND', id: command.id },
      },
    ]);
    render(<Inspector model={model} entity={command} issues={issues} onClose={vi.fn()} onSelect={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy fix prompt' }));
    await waitFor(() => expect(screen.getByText('Fix prompt copied')).toBeDefined());
    const copied = String(writeText.mock.calls[0]?.[0] ?? '');
    expect(copied).toMatch(/fix.*validation/i);
    expect(copied).toContain('COMMAND_MISSING_FIELD');
    expect(copied).toContain('The order identifier needs a source.');
    expect(copied).toContain('schema.fields.order_id');
    expect(copied).toContain('repair/context');
    expect(copied).toContain('/em/orders/order-service');
    expect(copied).toContain('"slug": "place"');
    expect(copied).toMatch(/rerun validation/i);
    expect(copied).not.toContain('do-not-copy');
  });
});

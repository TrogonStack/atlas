import { cleanup, fireEvent, render as rtlRender, screen, waitFor } from '@testing-library/react';
import { withNuqsTestingAdapter } from 'nuqs/adapters/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  api: {
    info: vi.fn(),
    namespaces: vi.fn(),
    model: vi.fn(),
    branches: vi.fn(),
    branchDiff: vi.fn(),
    search: vi.fn(),
    typeMessages: vi.fn(() => Promise.resolve({ messages: [] })),
  },
}));

vi.mock('@/lib/realtime', () => ({
  watchEntities: vi.fn(() => new Promise(() => {})),
}));

vi.mock('@/components/canvas/Board', () => ({
  Board: () => <div data-testid="board" />,
}));
vi.mock('@/components/canvas/SequenceBoard', () => ({
  SequenceBoard: () => <div data-testid="sequence-board" />,
}));
vi.mock('@/components/canvas/ScreensBoard', () => ({
  ScreensBoard: () => <div data-testid="screens-board" />,
}));
vi.mock('@/components/canvas/PlanBoard', () => ({
  PlanBoard: () => <div data-testid="plan-board" />,
}));
vi.mock('@/components/canvas/DomainChartBoard', () => ({
  DomainChartBoard: () => <div data-testid="domain-chart-board" />,
}));
vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>();
  return {
    ...actual,
    ReactFlowProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  };
});

// Intentionally NOT mocking @tanstack/react-hotkeys: this file proves Escape.

import { api } from '@/lib/api';
import { buildIssueIndex } from '@/lib/issues';
import { buildModel } from '@/lib/model';
import { ModelShell } from './ModelShell';

const eventWire = {
  event: {
    id: { namespace: 'shop', slug: 'order.placed', version: '1' },
    title: 'Order Placed',
  },
};

describe('ModelShell Escape hotkey', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (api.info as ReturnType<typeof vi.fn>).mockResolvedValue({});
    (api.namespaces as ReturnType<typeof vi.fn>).mockResolvedValue({ namespaces: ['shop'] });
    (api.model as ReturnType<typeof vi.fn>).mockResolvedValue({ entities: [eventWire] });
    (api.branches as ReturnType<typeof vi.fn>).mockResolvedValue({ branches: [] });
    (api.branchDiff as ReturnType<typeof vi.fn>).mockResolvedValue({ entries: [] });
    (api.search as ReturnType<typeof vi.fn>).mockResolvedValue({ results: [] });
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('closes the inspector on Escape after jumping from findings', async () => {
    const findingId = { namespace: 'shop', slug: 'order.shipped', version: '1' };
    const model = buildModel([eventWire, { event: { id: findingId, title: 'Order Shipped' } }] as never);
    const issues = buildIssueIndex([
      {
        severity: 'SEVERITY_ERROR',
        code: 'MISSING_FIELD',
        message: 'A field is missing.',
        subject: { kind: 'ENTITY_KIND_EVENT', id: findingId },
      },
    ]);
    rtlRender(<ModelShell model={model} scopedTitle="Shop" issues={issues} />, {
      wrapper: withNuqsTestingAdapter({ searchParams: '?selected=event:shop/order.placed@1' }),
    });
    fireEvent.click(screen.getByRole('button', { name: 'Validation findings' }));
    fireEvent.click(screen.getByRole('button', { name: 'Open event shop/order.shipped@1' }));
    expect(screen.getByRole('heading', { name: 'Order Shipped' })).toBeDefined();

    fireEvent.keyDown(document, { key: 'Escape', code: 'Escape', keyCode: 27 });
    await waitFor(() => {
      expect(screen.queryByRole('heading', { name: 'Order Shipped' })).toBeNull();
      expect(screen.queryByRole('heading', { name: 'Order Placed' })).toBeNull();
    });
  });

  it('closes validation on Escape while preserving the entity selected underneath', async () => {
    const model = buildModel([eventWire as never]);
    const issues = buildIssueIndex([
      { severity: 'SEVERITY_ERROR', code: 'MISSING_FIELD', message: 'A field is missing.' },
    ]);
    rtlRender(<ModelShell model={model} issues={issues} />, {
      wrapper: withNuqsTestingAdapter({ searchParams: `?selected=${encodeURIComponent(model.entities[0].key)}` }),
    });
    const trigger = screen.getByRole('button', { name: 'Validation findings' });
    fireEvent.click(trigger);
    expect(screen.getByRole('complementary', { name: 'Validation findings' })).toBeDefined();
    fireEvent.keyDown(document, { key: 'Escape', code: 'Escape', keyCode: 27 });
    await waitFor(() => {
      expect(screen.queryByRole('complementary', { name: 'Validation findings' })).toBeNull();
      expect(screen.getByRole('heading', { name: 'Order Placed' })).toBeDefined();
    });
    expect(document.activeElement).toBe(trigger);
  });

  // BUG: useHotkey('Escape', …, { ignoreInputs: true }) overrides the
  // library default for Escape (ignoreInputs: false). With the Sidebar
  // search focused, Esc becomes a dead key; the drawer stays open and the
  // input does not clear either (text inputs have no native Esc-clear).
  it('closes the inspector on Escape even when Sidebar search is focused', async () => {
    const model = buildModel([eventWire as never]);
    const key = model.entities[0].key;

    rtlRender(<ModelShell />, {
      wrapper: withNuqsTestingAdapter({ searchParams: `?selected=${encodeURIComponent(key)}` }),
    });

    await waitFor(() => {
      // Sidebar + Inspector are both <aside> (complementary).
      expect(screen.getAllByRole('complementary').length).toBe(2);
    });

    const search = screen.getByLabelText('Search the model');
    search.focus();
    expect(document.activeElement).toBe(search);

    fireEvent.keyDown(document, { key: 'Escape', code: 'Escape', keyCode: 27 });

    await waitFor(() => {
      expect(screen.getAllByRole('complementary').length).toBe(1);
    });
  });
});

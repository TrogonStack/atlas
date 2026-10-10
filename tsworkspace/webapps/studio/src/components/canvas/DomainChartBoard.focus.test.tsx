import { render } from '@testing-library/react';
import type React from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockSetCenter = vi.fn();
const mockGetZoom = vi.fn(() => 1);

vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>();
  return {
    ...actual,
    ReactFlow: (props: { children?: React.ReactNode }) => <div data-testid="react-flow">{props.children}</div>,
    ReactFlowProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
    useReactFlow: () => ({
      fitView: vi.fn(),
      setCenter: mockSetCenter,
      getZoom: mockGetZoom,
      getNodes: vi.fn(() => []),
    }),
    Background: () => null,
    Controls: () => null,
  };
});

import { buildModel } from '@/lib/model';
import { DomainChartBoard } from './DomainChartBoard';

const domainModel = buildModel([
  {
    domain: {
      id: { namespace: 'shop', slug: 'shop', version: '1' },
      title: 'Shop',
    },
  },
  {
    subdomain: {
      id: { namespace: 'shop', slug: 'checkout', version: '1' },
      title: 'Checkout',
      classification: 'core',
    },
  },
] as never);

const focusKey = domainModel.entities.find((e) => e.kind === 'subdomain')!.key;

describe('DomainChartBoard focus', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('pans the viewport to the focused subdomain (same contract as PlanBoard / Board)', () => {
    render(<DomainChartBoard model={domainModel} selections={[]} onSelect={vi.fn()} focus={{ key: focusKey, n: 1 }} />);

    // Public contract: ModelShell passes focus={focusReq} into DomainChartBoard
    // so a Domain-tab deep-link / sidebar selectAndFocus recenters the chart.
    expect(mockSetCenter).toHaveBeenCalled();
  });
});

import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { EventModelPage } from './EventModelPage';

vi.mock('@/lib/api', () => ({
  api: {
    eventModel: vi.fn(),
    validate: vi.fn(),
  },
}));

let capturedOnRefetch: (() => void) | undefined;
let capturedIssues: IssueIndex | undefined;

vi.mock('@/components/ModelShell', () => ({
  ModelShell: ({
    scopedTitle,
    issues,
    onRefetch,
  }: {
    scopedTitle?: string;
    issues?: IssueIndex;
    onRefetch?: () => void;
  }) => {
    capturedOnRefetch = onRefetch;
    capturedIssues = issues;
    return <div data-testid="model-shell">{scopedTitle ?? 'shell'}</div>;
  },
}));

import { api } from '@/lib/api';
import type { IssueIndex } from '@/lib/issues';

describe('EventModelPage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    capturedOnRefetch = undefined;
    capturedIssues = undefined;
    (api.validate as ReturnType<typeof vi.fn>).mockResolvedValue({ issues: [] });
    window.history.replaceState({}, '', '/em/shop/buy-flow');
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows a loading state while fetching', () => {
    (api.eventModel as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    render(<EventModelPage namespace="shop" slug="buy-flow" />);
    expect(screen.getByText(/Loading shop\/buy-flow/)).toBeDefined();
  });

  it('shows the ModelShell on success', async () => {
    (api.eventModel as ReturnType<typeof vi.fn>).mockResolvedValue({ entities: [] });
    render(<EventModelPage namespace="shop" slug="buy-flow" />);
    await waitFor(() => {
      expect(screen.getByTestId('model-shell')).toBeDefined();
    });
  });

  it('shows an error state when the fetch rejects', async () => {
    (api.eventModel as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('not found'));
    render(<EventModelPage namespace="shop" slug="missing" />);
    await waitFor(() => {
      expect(screen.getByText('not found')).toBeDefined();
    });
    expect(screen.getByText('Could not load event model')).toBeDefined();
    expect(screen.getByRole('link', { name: /back to overview/i })).toBeDefined();
  });

  it('does not update state after unmount (aborts in-flight fetch)', async () => {
    let resolveModel!: (v: { entities: unknown[] }) => void;
    (api.eventModel as ReturnType<typeof vi.fn>).mockReturnValue(
      new Promise<{ entities: unknown[] }>((r) => {
        resolveModel = r;
      }),
    );
    const { unmount } = render(<EventModelPage namespace="shop" slug="buy-flow" />);
    unmount();
    expect(() => resolveModel({ entities: [] })).not.toThrow();
  });

  // EventModelPage closes `branch` into `load` from a non-reactive
  // readBranchFromUrl() call. After BranchBar updates ?branch= (child
  // re-renders via nuqs; parent does not), the first onRefetch still
  // ships the stale branch. Assert the immediate call: a later
  // setModel-driven re-render may rebuild load and mask the bug.
  it('onRefetch uses a stale branch closure after ?branch= changes without remount', async () => {
    (api.eventModel as ReturnType<typeof vi.fn>).mockResolvedValue({ entities: [] });
    render(<EventModelPage namespace="shop" slug="buy-flow" />);
    await waitFor(() => {
      expect(screen.getByTestId('model-shell')).toBeDefined();
    });

    window.history.replaceState({}, '', '/em/shop/buy-flow?branch=alex/rework');
    (api.eventModel as ReturnType<typeof vi.fn>).mockClear();
    expect(capturedOnRefetch).toBeTypeOf('function');
    capturedOnRefetch!();

    expect(api.eventModel).toHaveBeenCalledTimes(1);
    expect(api.eventModel).toHaveBeenCalledWith('shop', 'buy-flow', expect.objectContaining({ branch: 'alex/rework' }));
  });
});

// The validator's findings existed server-side long before anything in the
// Studio asked for them. This page is where the ask lives, because
// ValidateEventModel is scoped to one EventModel id and only the /em/ route
// knows which one is open.
describe('EventModelPage: validation', () => {
  // This file registers no global auto-cleanup, so a leftover render from an
  // earlier test makes getByTestId ambiguous rather than failing loudly.
  beforeEach(() => {
    cleanup();
    vi.clearAllMocks();
    capturedIssues = undefined;
    window.history.replaceState({}, '', '/em/registry/changeset-review-flow');
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  const finding = {
    severity: 'SEVERITY_WARNING',
    code: 'RM_EVENT_NOT_PROJECTED',
    message: 'no in-model read-model slice projects it',
    subject: {
      kind: 'ENTITY_KIND_READ_MODEL',
      id: { namespace: 'registry', slug: 'changesets-awaiting-review', version: '1' },
    },
    subjectField: 'source_events',
    ruleTitle: 'Read model declares an event nothing projects',
    ruleCategory: 'model',
    ruleDefaultSeverity: 'SEVERITY_WARNING',
  };

  it('validates the open event model and hands the findings to the shell', async () => {
    (api.eventModel as ReturnType<typeof vi.fn>).mockResolvedValue({ entities: [] });
    (api.validate as ReturnType<typeof vi.fn>).mockResolvedValue({ issues: [finding] });
    render(<EventModelPage namespace="registry" slug="changeset-review-flow" />);

    await waitFor(() => {
      expect(capturedIssues?.all.length).toBe(1);
    });
    expect(api.validate).toHaveBeenCalledWith(
      { namespace: 'registry', slug: 'changeset-review-flow' },
      expect.objectContaining({ signal: expect.anything() }),
    );
    expect(
      capturedIssues?.for({
        kind: 'readModel',
        id: { namespace: 'registry', slug: 'changesets-awaiting-review', version: '1' },
      })[0].code,
    ).toBe('RM_EVENT_NOT_PROJECTED');
  });

  it('still renders the board when validation fails, and reports why', async () => {
    (api.eventModel as ReturnType<typeof vi.fn>).mockResolvedValue({ entities: [] });
    (api.validate as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('validate: 503'));
    render(<EventModelPage namespace="registry" slug="changeset-review-flow" />);

    await waitFor(() => {
      expect(screen.getByTestId('model-shell')).toBeDefined();
    });
    await waitFor(() => {
      expect(capturedIssues?.error).toBe('validate: 503');
    });
  });

  it('does not block the board on a validator that never answers', async () => {
    (api.eventModel as ReturnType<typeof vi.fn>).mockResolvedValue({ entities: [] });
    (api.validate as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    render(<EventModelPage namespace="registry" slug="changeset-review-flow" />);
    await waitFor(() => {
      expect(screen.getByTestId('model-shell')).toBeDefined();
    });
  });
});

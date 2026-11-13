import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  api,
  compareGlossaries,
  getGlossaryTerm,
  getGlossaryUsage,
  inspectGlossaryRefresh,
  inspectGlossarySources,
  listGlossaryEntries,
} from '@/lib/api';
import { GlossaryPage } from './GlossaryPage';
import type { GlossaryIndexEntry, GlossaryTermDetail } from '@/lib/glossary';

vi.mock('@/lib/api', () => ({
  api: { namespaces: vi.fn() },
  listGlossaryEntries: vi.fn(),
  getGlossaryTerm: vi.fn(),
  getGlossaryUsage: vi.fn(),
  inspectGlossarySources: vi.fn(),
  compareGlossaries: vi.fn(),
  inspectGlossaryRefresh: vi.fn(),
}));

const id = { namespace: 'sales', slug: 'order', version: '9007199254740993' };
const source = { id: { namespace: 'sales', slug: 'handbook', version: '3' } };
const entry = {
  term: { id },
  title: 'Order',
  audience: 'internal',
  state: 'GLOSSARY_ENTRY_STATE_SOURCE_CHANGED',
  published: false,
  ownerUid: 'term-owner',
  etag: 'term-etag',
} satisfies GlossaryIndexEntry;
const detail = {
  entry,
  etag: 'term-etag',
  entity: {
    system: { uid: 'term-owner' },
    term: {
      id,
      title: 'Order',
      doc: 'A customer request awaiting fulfillment.',
      embodiedBy: [{ kind: 'ENTITY_KIND_COMMAND', id: { namespace: 'sales', slug: 'place-order', version: '2' } }],
      glossary: {
        audience: 'internal',
        generationRevision: 'extract-v1',
        nextReviewOn: { value: '2026-12-01' },
        evidence: [
          {
            document: source,
            ownerUid: 'document-owner',
            contentDigest: { sha256: 'a'.repeat(64) },
            locator: 'paragraph-2',
            quote: '<script>Orders are requests.</script>',
          },
        ],
        review: {
          actor: 'reviewer',
          reviewedAt: '2026-11-12T12:00:00Z',
          disposition: 'GLOSSARY_REVIEW_DISPOSITION_ACCEPTED',
          reason: 'Definition corrected.',
        },
        policy: {
          usage: 'GLOSSARY_USAGE_DEPRECATED',
          preferredTerm: { id: { ...id, slug: 'purchase-request', version: '7' } },
          rationale: 'Use customer language in the storefront.',
        },
      },
    },
  },
  evidence: [{ evidenceIndex: 0, state: 'GLOSSARY_EVIDENCE_STATE_CHANGED', reason: 'Source digest changed.' }],
} satisfies GlossaryTermDetail;

beforeEach(() => {
  vi.clearAllMocks();
  window.history.replaceState(null, '', '/glossary');
  vi.mocked(api.namespaces).mockResolvedValue({
    namespaces: ['sales', 'shipping'],
    labels: { sales: 'Sales', shipping: 'Shipping' },
  });
  vi.mocked(listGlossaryEntries).mockResolvedValue({
    entries: [entry],
    snapshotRevision: 'snapshot-one',
    incomplete: false,
  });
  vi.mocked(getGlossaryTerm).mockResolvedValue(detail);
  vi.mocked(getGlossaryUsage).mockResolvedValue({
    entity: {
      command: {
        id: { namespace: 'sales', slug: 'place-order', version: '2' },
        title: 'Place purchase request',
        doc: 'Validate the cart before accepting the request.',
      },
    },
    etag: 'command-etag',
  });
  vi.mocked(inspectGlossarySources).mockResolvedValue({
    sources: [{ document: source, text: 'Current source text.', state: 'GLOSSARY_EVIDENCE_STATE_CURRENT' }],
  });
  vi.mocked(compareGlossaries).mockResolvedValue({
    differences: [
      {
        kind: 'GLOSSARY_DIFFERENCE_KIND_DIFFERENT_MEANING',
        left: { id },
        right: { id: { namespace: 'shipping', slug: 'order', version: '4' } },
        title: 'Order',
        explanation: 'The contexts use order differently.',
      },
    ],
    incomplete: true,
    snapshotRevision: 'comparison-revision',
  });
  vi.mocked(inspectGlossaryRefresh).mockResolvedValue({
    observations: [{ entry, generationChanged: true }],
    incomplete: false,
    snapshotRevision: 'refresh-revision',
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  Reflect.deleteProperty(navigator, 'clipboard');
});

async function chooseSales() {
  await screen.findByRole('option', { name: 'Sales' });
  fireEvent.change(screen.getByRole('combobox', { name: 'Bounded context' }), { target: { value: 'sales' } });
}

describe('glossary review page', () => {
  it('starts with explicit context selection rather than claiming an empty glossary', () => {
    render(<GlossaryPage />);
    expect(screen.getByRole('heading', { name: 'Glossary', level: 1 })).toBeTruthy();
    expect(screen.getByRole('combobox', { name: 'Bounded context' })).toBeTruthy();
    expect(screen.getByText('Choose a bounded context to inspect its vocabulary.')).toBeTruthy();
    expect(screen.queryByText('No glossary entries.')).toBeNull();
    expect(screen.queryByRole('button', { name: /^(approve|publish|save|delete)(?:\s|$)/i })).toBeNull();
  });

  it('inspects exact vocabulary, authoritative review/freshness, evidence and correction context without mutations', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    render(<GlossaryPage />);
    await chooseSales();
    fireEvent.click(await screen.findByRole('button', { name: /^Order/ }));
    expect(await screen.findByText('A customer request awaiting fulfillment.')).toBeTruthy();
    expect(screen.getAllByText('Source changed').length).toBeGreaterThan(0);
    expect(screen.getByText('Changed')).toBeTruthy();
    expect(screen.getByText('Not in published selection')).toBeTruthy();
    expect(screen.getByText('reviewer')).toBeTruthy();
    expect(screen.getByText('<script>Orders are requests.</script>')).toBeTruthy();
    expect(document.querySelector('script')).toBeNull();
    expect(screen.getByText('sales/place-order@2')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'sales/place-order@2' }));
    expect(await screen.findByText('Place purchase request')).toBeTruthy();
    expect(screen.getByText('Validate the cart before accepting the request.')).toBeTruthy();
    expect(getGlossaryUsage).toHaveBeenCalledWith(
      { kind: 'ENTITY_KIND_COMMAND', id: { namespace: 'sales', slug: 'place-order', version: '2' } },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    expect(screen.getByRole('link', { name: 'sales/purchase-request@7' }).getAttribute('href')).toContain('version=7');
    fireEvent.click(screen.getByRole('button', { name: 'Inspect source' }));
    expect(await screen.findByText('Current source text.')).toBeTruthy();
    expect(screen.getByText('Current')).toBeTruthy();
    expect(inspectGlossarySources).toHaveBeenCalledWith(
      { documents: [source] },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Copy correction context' }));
    await waitFor(() => expect(writeText).toHaveBeenCalled());
    const copied = writeText.mock.calls[0][0];
    expect(copied).toContain('sales/order@9007199254740993');
    expect(copied).toContain('term-etag');
    expect(copied).toContain('Source changed');
    expect(screen.queryByRole('button', { name: /^(approve|publish|save|delete)(?:\s|$)/i })).toBeNull();
    expect(getGlossaryTerm).toHaveBeenCalledWith({ term: { id } }, expect.anything());
  });

  it('retains partial-catalog status and follows the backend page token in the same context', async () => {
    vi.mocked(listGlossaryEntries)
      .mockResolvedValueOnce({
        entries: [entry],
        nextPageToken: 'opaque-page',
        incomplete: true,
        snapshotRevision: 'page-revision',
      })
      .mockResolvedValueOnce({ entries: [], incomplete: false, snapshotRevision: 'page-revision' });
    render(<GlossaryPage />);
    await chooseSales();
    expect(
      await screen.findByText('The glossary scan is incomplete. Missing entries do not establish absence.'),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Next page' }));
    await waitFor(() => expect(listGlossaryEntries).toHaveBeenCalledTimes(2));
    expect(vi.mocked(listGlossaryEntries).mock.calls[1][0]).toEqual({
      selection: { namespaces: ['sales'], includeUnreviewed: true },
      pageSize: 100,
      pageToken: 'opaque-page',
    });
  });

  it('does not substitute another version when modeled usage inspection returns a mismatched resource', async () => {
    vi.mocked(getGlossaryUsage).mockResolvedValue({
      entity: {
        command: { id: { namespace: 'sales', slug: 'place-order', version: '3' }, title: 'Unselected latest version' },
      },
    });
    render(<GlossaryPage />);
    await chooseSales();
    fireEvent.click(await screen.findByRole('button', { name: /^Order/ }));
    fireEvent.click(await screen.findByRole('button', { name: 'sales/place-order@2' }));
    expect(await screen.findByText('The response does not match the exact modeled reference.')).toBeTruthy();
    expect(screen.queryByText('Unselected latest version')).toBeNull();
  });

  it('shows operational failure without an empty-catalog claim', async () => {
    vi.mocked(listGlossaryEntries).mockRejectedValue(new Error('catalog unavailable'));
    render(<GlossaryPage />);
    await chooseSales();
    expect(await screen.findByRole('alert')).toHaveProperty(
      'textContent',
      expect.stringContaining('catalog unavailable'),
    );
    expect(screen.queryByText('No glossary entries.')).toBeNull();
  });

  it('compares bounded contexts without treating a difference as a ruling', async () => {
    render(<GlossaryPage />);
    await chooseSales();
    fireEvent.change(screen.getByRole('combobox', { name: 'Compare with context' }), { target: { value: 'shipping' } });
    fireEvent.click(screen.getByRole('button', { name: 'Compare contexts' }));
    expect(await screen.findByText('The contexts use order differently.')).toBeTruthy();
    expect(screen.getByText('Comparison is incomplete.')).toBeTruthy();
    expect(screen.getByText('Differences are candidates for review, not language rulings.')).toBeTruthy();
    expect(compareGlossaries).toHaveBeenCalledWith(
      {
        left: { namespaces: ['sales'], includeUnreviewed: true },
        right: { namespaces: ['shipping'], includeUnreviewed: true },
      },
      expect.anything(),
    );
  });

  it('reopens an exact large-version deep link in the active branch', async () => {
    window.history.replaceState(
      null,
      '',
      '/glossary?namespace=sales&slug=order&version=9007199254740993&branch=review',
    );
    render(<GlossaryPage />);
    expect(await screen.findByText('A customer request awaiting fulfillment.')).toBeTruthy();
    expect(getGlossaryTerm).toHaveBeenCalledWith({ term: { id } }, expect.objectContaining({ branch: 'review' }));
  });

  it('does not hydrate a live glossary behind a historical URL', async () => {
    window.history.replaceState(null, '', '/glossary?namespace=sales&asOf=past');
    render(<GlossaryPage />);
    expect(
      await screen.findByText(
        'Historical glossary inspection is unavailable. Return to the live glossary to inspect current evidence.',
      ),
    ).toBeTruthy();
    expect(listGlossaryEntries).not.toHaveBeenCalled();
    expect(getGlossaryTerm).not.toHaveBeenCalled();
  });
});

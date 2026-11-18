import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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
    expect(await screen.findByText('No entries on this page.')).toBeTruthy();
    expect(screen.queryByText('No glossary entries.')).toBeNull();
  });

  it('reloads the selected exact detail along with the catalog', async () => {
    render(<GlossaryPage />);
    await chooseSales();
    fireEvent.click(await screen.findByRole('button', { name: /^Order/ }));
    expect(await screen.findByText('A customer request awaiting fulfillment.')).toBeTruthy();
    vi.mocked(getGlossaryTerm).mockResolvedValue({
      ...detail,
      etag: 'updated-term-etag',
      entity: { ...detail.entity, term: { ...detail.entity.term, doc: 'The reviewed definition has changed.' } },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Reload' }));
    expect(await screen.findByText('The reviewed definition has changed.')).toBeTruthy();
    expect(screen.queryByText('A customer request awaiting fulfillment.')).toBeNull();
    expect(getGlossaryTerm).toHaveBeenCalledTimes(2);
    expect(getGlossaryTerm).toHaveBeenLastCalledWith({ term: { id } }, expect.anything());
    expect(listGlossaryEntries).toHaveBeenCalledTimes(2);
  });

  it('rechecks freshness for the same generation revision', async () => {
    render(<GlossaryPage />);
    await chooseSales();
    fireEvent.click(await screen.findByRole('button', { name: /^Order/ }));
    fireEvent.click(await screen.findByText('Context refresh inspection'));
    fireEvent.click(screen.getByRole('button', { name: 'Check freshness' }));
    await waitFor(() => expect(inspectGlossaryRefresh).toHaveBeenCalledTimes(1));
    expect(await screen.findByText(/Generation revision changed/)).toBeTruthy();
    vi.mocked(inspectGlossaryRefresh).mockResolvedValue({ observations: [], incomplete: true });
    fireEvent.click(screen.getByRole('button', { name: 'Check freshness' }));
    expect(await screen.findByText('Refresh inspection is incomplete.')).toBeTruthy();
    expect(inspectGlossaryRefresh).toHaveBeenCalledTimes(2);
    expect(vi.mocked(inspectGlossaryRefresh).mock.calls[1][0].generationRevision).toBe('extract-v1');
  });

  it('refreshes a source inspection when opened again', async () => {
    window.history.replaceState(null, '', '/glossary?namespace=sales&branch=review');
    render(<GlossaryPage />);
    fireEvent.click(await screen.findByRole('button', { name: /^Order/ }));
    fireEvent.click(await screen.findByRole('button', { name: 'Inspect source' }));
    expect(await screen.findByText('Current source text.')).toBeTruthy();
    vi.mocked(inspectGlossarySources).mockResolvedValue({
      sources: [{ document: source, text: 'Updated source text.', state: 'GLOSSARY_EVIDENCE_STATE_CURRENT' }],
    });
    fireEvent.click(screen.getByRole('button', { name: 'Inspect source' }));
    expect(await screen.findByText('Updated source text.')).toBeTruthy();
    expect(screen.queryByText('Current source text.')).toBeNull();
    expect(inspectGlossarySources).toHaveBeenCalledTimes(2);
    expect(inspectGlossarySources).toHaveBeenLastCalledWith(
      { documents: [source] },
      expect.objectContaining({ branch: 'review', signal: expect.any(AbortSignal) }),
    );
  });

  it('refreshes a modeled-resource inspection when opened again', async () => {
    window.history.replaceState(null, '', '/glossary?namespace=sales&branch=review');
    render(<GlossaryPage />);
    fireEvent.click(await screen.findByRole('button', { name: /^Order/ }));
    fireEvent.click(await screen.findByRole('button', { name: 'sales/place-order@2' }));
    expect(await screen.findByText('Place purchase request')).toBeTruthy();
    vi.mocked(getGlossaryUsage).mockResolvedValue({
      entity: {
        command: {
          id: { namespace: 'sales', slug: 'place-order', version: '2' },
          title: 'Updated purchase request',
          doc: 'The modeled resource changed.',
        },
      },
      etag: 'updated-command-etag',
    });
    fireEvent.click(screen.getByRole('button', { name: 'sales/place-order@2' }));
    expect(await screen.findByText('Updated purchase request')).toBeTruthy();
    expect(screen.queryByText('Place purchase request')).toBeNull();
    expect(getGlossaryUsage).toHaveBeenCalledTimes(2);
    expect(getGlossaryUsage).toHaveBeenLastCalledWith(
      { kind: 'ENTITY_KIND_COMMAND', id: { namespace: 'sales', slug: 'place-order', version: '2' } },
      expect.objectContaining({ branch: 'review', signal: expect.any(AbortSignal) }),
    );
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
    fireEvent.click(screen.getByRole('button', { name: 'Compare contexts' }));
    await waitFor(() => expect(compareGlossaries).toHaveBeenCalledTimes(2));
  });

  it('compares independently selected audiences and clears a superseded comparison', async () => {
    render(<GlossaryPage />);
    await chooseSales();
    fireEvent.change(screen.getByRole('textbox', { name: 'Audience' }), { target: { value: 'internal' } });
    fireEvent.change(screen.getByRole('combobox', { name: 'Compare with context' }), { target: { value: 'shipping' } });
    fireEvent.change(screen.getByRole('textbox', { name: 'Comparison audience' }), { target: { value: 'customer' } });
    fireEvent.click(screen.getByRole('button', { name: 'Compare contexts' }));
    expect(await screen.findByText('The contexts use order differently.')).toBeTruthy();
    expect(compareGlossaries).toHaveBeenLastCalledWith(
      {
        left: { namespaces: ['sales'], audience: 'internal', includeUnreviewed: true },
        right: { namespaces: ['shipping'], audience: 'customer', includeUnreviewed: true },
      },
      expect.anything(),
    );
    fireEvent.change(screen.getByRole('textbox', { name: 'Comparison audience' }), { target: { value: 'code' } });
    expect(screen.queryByText('The contexts use order differently.')).toBeNull();
    expect(compareGlossaries).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'Compare contexts' }));
    await waitFor(() => expect(compareGlossaries).toHaveBeenCalledTimes(2));
    expect(vi.mocked(compareGlossaries).mock.calls[1][0].right).toEqual({
      namespaces: ['shipping'],
      audience: 'code',
      includeUnreviewed: true,
    });
  });

  it('compares code and human vocabulary inside the same bounded context', async () => {
    render(<GlossaryPage />);
    await chooseSales();
    fireEvent.change(screen.getByRole('textbox', { name: 'Audience' }), { target: { value: 'internal' } });
    const target = screen.getByRole('combobox', { name: 'Compare with context' });
    expect(within(target).getByRole('option', { name: 'Sales' })).toBeTruthy();
    fireEvent.change(target, { target: { value: 'sales' } });
    fireEvent.change(screen.getByRole('textbox', { name: 'Comparison audience' }), { target: { value: 'code' } });
    fireEvent.click(screen.getByRole('button', { name: 'Compare contexts' }));
    await waitFor(() => expect(compareGlossaries).toHaveBeenCalled());
    expect(vi.mocked(compareGlossaries).mock.calls[0][0]).toEqual({
      left: { namespaces: ['sales'], audience: 'internal', includeUnreviewed: true },
      right: { namespaces: ['sales'], audience: 'code', includeUnreviewed: true },
    });
  });

  it('keeps the permitted term period distinct from terminology-rule activation dates', async () => {
    vi.mocked(getGlossaryTerm).mockResolvedValue({
      ...detail,
      entity: {
        ...detail.entity,
        term: {
          ...detail.entity.term,
          glossary: {
            ...detail.entity.term.glossary,
            usagePeriod: { validFrom: { value: '2026-11-01' }, validUntil: { value: '2026-12-01' } },
            policy: {
              ...detail.entity.term.glossary.policy,
              effectiveFrom: { value: '2026-11-15' },
              effectiveUntil: { value: '2026-11-20' },
            },
          },
        },
      },
    });
    render(<GlossaryPage />);
    await chooseSales();
    fireEvent.click(await screen.findByRole('button', { name: /^Order/ }));
    const period = await screen.findByRole('region', { name: 'Permitted term use' });
    expect(period.textContent).toContain('2026-11-01');
    expect(period.textContent).toContain('2026-12-01');
    expect(period.textContent).toContain('end excluded');
    expect(screen.getByText('Effective from 2026-11-15 until 2026-11-20 (end excluded).')).toBeTruthy();
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

// Domain Chart invariants. The chart is a derived view of the
// problem-space knowledge graph (Decision #29): subdomains sit inside
// the band their `classification` names, bounded contexts hang under
// the subdomains they `realize`, and the "Unmapped" band only fires
// when there is something to flag.
import { describe, expect, it } from 'vitest';
import {
  buildDomainChart,
  type ChartMarker,
  type ContextCardData,
  chartMarkers,
  markerTooltip,
  SPLIT_CANDIDATE_CODE,
  splitCandidateText,
  validationScopes,
} from './domainchart';
import type { Issue } from './issues';
import { buildModel } from './model';

const id = (namespace: string, slug: string): Record<string, unknown> => ({
  namespace,
  slug,
  version: '1',
});

function fixtureEntities(): Record<string, unknown>[] {
  return [
    { domain: { id: id('shop', 'shop'), title: 'shop' } },
    {
      subdomain: {
        id: id('shop', 'identity'),
        title: 'identity',
        domain: { id: id('shop', 'shop') },
        classification: 'CORE',
      },
    },
    {
      subdomain: {
        id: id('shop', 'billing'),
        title: 'billing',
        domain: { id: id('shop', 'shop') },
        classification: 'SUPPORTING',
      },
    },
    {
      boundedContext: {
        id: id('accounts', 'accounts'),
        title: 'accounts',
        realizes: [{ id: id('shop', 'identity') }],
      },
    },
  ];
}

describe('buildDomainChart', () => {
  it('places a subdomain card under the band matching its classification', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes } = buildDomainChart(model);
    const coreBand = nodes.find((n) => n.id === 'band:core');
    const supportingBand = nodes.find((n) => n.id === 'band:supporting');
    const identitySd = nodes.find((n) => n.id.startsWith('sd:') && n.id.includes('identity'));
    const billingSd = nodes.find((n) => n.id.startsWith('sd:') && n.id.includes('billing'));
    expect(coreBand).toBeDefined();
    expect(supportingBand).toBeDefined();
    expect(identitySd).toBeDefined();
    expect(billingSd).toBeDefined();
    // The subdomain card's y must fall inside its band's vertical extent.
    const inBand = (
      sd: { position: { y: number } },
      band: { position: { y: number }; style?: Record<string, unknown> },
    ) => {
      const h = Number((band.style as { height?: number })?.height ?? 0);
      return sd.position.y >= band.position.y && sd.position.y <= band.position.y + h;
    };
    expect(inBand(identitySd as never, coreBand as never)).toBe(true);
    expect(inBand(billingSd as never, supportingBand as never)).toBe(true);
  });

  it('emits a realizes edge from the context card up to its subdomain', () => {
    const model = buildModel(fixtureEntities() as never);
    const { edges } = buildDomainChart(model);
    const realized = edges.filter((e) => (e.data as { relation?: string })?.relation === 'realizes');
    expect(realized.length).toBe(1);
    const e = realized[0];
    expect(e.source).toMatch(/^ctx:/);
    expect(e.target).toMatch(/^sd:/);
  });

  it('does not render an Unmapped band when every context realizes something', () => {
    const model = buildModel(fixtureEntities() as never);
    const { nodes } = buildDomainChart(model);
    expect(nodes.some((n) => n.id === 'band:unmapped')).toBe(false);
  });

  it('renders an Unmapped band when at least one context realizes nothing', () => {
    const entities = fixtureEntities();
    entities.push({
      boundedContext: {
        id: id('orphan', 'orphan'),
        title: 'orphan',
      },
    });
    const model = buildModel(entities as never);
    const { nodes } = buildDomainChart(model);
    expect(nodes.some((n) => n.id === 'band:unmapped' || n.type === 'unmappedBand')).toBe(true);
  });

  // BUG: unmapped band width is hard-coded to 3*(CARD_W+COL_GAP) while
  // cards are laid out at BAND_PAD + i*(CTX_W+COL_GAP), so a 4th orphan
  // overflows the band.
  it('sizes the Unmapped band wide enough for every unmapped context card', () => {
    const entities = fixtureEntities();
    for (const slug of ['o1', 'o2', 'o3', 'o4']) {
      entities.push({
        boundedContext: { id: id('orphan', slug), title: slug },
      });
    }
    const model = buildModel(entities as never);
    const { nodes } = buildDomainChart(model);
    const band = nodes.find((n) => n.id === 'band:unmapped');
    expect(band).toBeDefined();
    const bandW = Number((band?.style as { width?: number })?.width ?? 0);
    const cards = nodes.filter((n) => n.id.startsWith('ctx:unmapped:'));
    expect(cards.length).toBe(4);
    for (const card of cards) {
      const right = card.position.x + Number((card.style as { width?: number })?.width ?? 0);
      expect(right).toBeLessThanOrEqual(bandW);
    }
  });

  describe('contexts realizing several subdomains', () => {
    const sharedEntities = (secondClassification: string): Record<string, unknown>[] => [
      ...fixtureEntities(),
      {
        subdomain: {
          id: id('shop', 'payments'),
          title: 'payments',
          domain: { id: id('shop', 'shop') },
          classification: secondClassification,
        },
      },
      {
        boundedContext: {
          id: id('storefront', 'storefront'),
          title: 'storefront',
          realizes: [{ id: id('shop', 'identity') }, { id: id('shop', 'payments') }],
        },
      },
    ];

    const copiesOf = (nodes: { id: string; data: unknown }[], title: string) =>
      nodes.filter((n) => n.id.startsWith('ctx:') && (n.data as ContextCardData).entity.title === title);

    const mixedFinding = (slug: string): Issue => ({
      severity: 'warning',
      code: SPLIT_CANDIDATE_CODE,
      message: 'realizes subdomains of different classifications',
      field: 'realizes',
      ruleTitle: '',
      ruleCategory: 'strategic',
      subject: { kind: 'boundedContext', id: { namespace: slug, slug, version: '1' } },
    });

    it('lists the other subdomains on each copy', () => {
      const { nodes } = buildDomainChart(buildModel(sharedEntities('SUPPORTING') as never));
      const copies = copiesOf(nodes, 'storefront').map((n) => n.data as ContextCardData);
      expect(copies.length).toBe(2);
      const underCore = copies.find((c) => c.classification === 'core');
      const underSupporting = copies.find((c) => c.classification === 'supporting');
      expect(underCore?.alsoRealizes.map((o) => [o.title, o.classification])).toEqual([['payments', 'supporting']]);
      expect(underSupporting?.alsoRealizes.map((o) => [o.title, o.classification])).toEqual([['identity', 'core']]);
    });

    it('does not decide a split candidate without the server finding', () => {
      const { nodes } = buildDomainChart(buildModel(sharedEntities('SUPPORTING') as never));
      for (const copy of copiesOf(nodes, 'storefront')) {
        expect((copy.data as ContextCardData).splitCandidate).toBeUndefined();
      }
    });

    it('flags a split candidate on every copy when the server reports mixed classifications', () => {
      const markers = chartMarkers([mixedFinding('storefront')]);
      const { nodes } = buildDomainChart(buildModel(sharedEntities('SUPPORTING') as never), markers);
      const copies = copiesOf(nodes, 'storefront').map((n) => n.data as ContextCardData);
      expect(copies.map((c) => c.splitCandidate)).toEqual([
        'split candidate: core and supporting',
        'split candidate: core and supporting',
      ]);
      expect(copies.every((c) => c.marker?.findings[0]?.code === SPLIT_CANDIDATE_CODE)).toBe(true);
    });

    it('shows a marker without a split chip when the realized subdomains share a band', () => {
      const markers = chartMarkers([mixedFinding('storefront')]);
      const { nodes } = buildDomainChart(buildModel(sharedEntities('CORE') as never), markers);
      for (const copy of copiesOf(nodes, 'storefront')) {
        const data = copy.data as ContextCardData;
        expect(data.alsoRealizes.length).toBe(1);
        expect(data.splitCandidate).toBeUndefined();
        expect(data.marker).toBeDefined();
      }
    });

    it('leaves a single-subdomain context neither shared nor spanning', () => {
      const { nodes } = buildDomainChart(buildModel(fixtureEntities() as never));
      const copies = copiesOf(nodes, 'accounts');
      expect(copies.length).toBe(1);
      const data = copies[0].data as ContextCardData;
      expect(data.alsoRealizes).toEqual([]);
      expect(data.splitCandidate).toBeUndefined();
    });

    it('gives unmapped contexts no other subdomains', () => {
      const entities = [...fixtureEntities(), { boundedContext: { id: id('orphan', 'orphan'), title: 'orphan' } }];
      const { nodes } = buildDomainChart(buildModel(entities as never));
      const data = nodes.find((n) => n.id.startsWith('ctx:unmapped:'))?.data as ContextCardData;
      expect(data.alsoRealizes).toEqual([]);
      expect(data.splitCandidate).toBeUndefined();
    });
  });

  describe('splitCandidateText', () => {
    it('names the two spanned bands', () => {
      expect(splitCandidateText(['core', 'generic'])).toBe('split candidate: core and generic');
    });

    it('lists three spanned bands in order', () => {
      expect(splitCandidateText(['core', 'supporting', 'generic'])).toBe(
        'split candidate: core, supporting and generic',
      );
    });
  });
});

function strategicIssue(overrides: Partial<Issue> = {}): Issue {
  return {
    severity: 'warning',
    code: 'DOMAIN_IN_CONTEXT_NAMESPACE',
    message: 'Domain declared in a context namespace.',
    field: '',
    ruleTitle: '',
    ruleCategory: 'strategic',
    subject: { kind: 'domain', id: { namespace: 'accounts', slug: 'accounts', version: '1' } },
    ...overrides,
  };
}

describe('chartMarkers', () => {
  it('groups findings by entity, worst first, and reports the worst severity', () => {
    const markers = chartMarkers([
      strategicIssue({ severity: 'info', code: 'B_INFO' }),
      strategicIssue({ severity: 'error', code: 'A_ERROR' }),
      strategicIssue({ severity: 'warning', code: 'A_WARN' }),
    ]);
    const marker = markers.get('domain:accounts/accounts@1') as ChartMarker;
    expect(marker.severity).toBe('error');
    expect(marker.findings.map((f) => f.code)).toEqual(['A_ERROR', 'A_WARN', 'B_INFO']);
    expect(markerTooltip(marker)).toBe(
      'A_ERROR: Domain declared in a context namespace.\nA_WARN: Domain declared in a context namespace.\nB_INFO: Domain declared in a context namespace.',
    );
  });

  it('ignores findings without a subject or on kinds the chart does not draw', () => {
    const markers = chartMarkers([
      strategicIssue({ subject: undefined }),
      strategicIssue({ subject: { kind: 'event', id: { namespace: 'shop', slug: 'placed', version: '1' } } }),
    ]);
    expect(markers.size).toBe(0);
  });

  it('puts the marker on the domain header and the subdomain card', () => {
    const markers = chartMarkers([
      strategicIssue({ subject: { kind: 'domain', id: { namespace: 'shop', slug: 'shop', version: '1' } } }),
      strategicIssue({
        code: 'SUBDOMAIN_UNREALIZED',
        subject: { kind: 'subdomain', id: { namespace: 'shop', slug: 'billing', version: '1' } },
      }),
    ]);
    const { nodes } = buildDomainChart(buildModel(fixtureEntities() as never), markers);
    const header = nodes.find((n) => n.type === 'domainHeader')?.data as { marker?: ChartMarker };
    const billing = nodes.find((n) => n.id.startsWith('sd:') && n.id.includes('billing'))?.data as {
      marker?: ChartMarker;
    };
    expect(header.marker?.findings[0].code).toBe('DOMAIN_IN_CONTEXT_NAMESPACE');
    expect(billing.marker?.findings[0].code).toBe('SUBDOMAIN_UNREALIZED');
  });
});

describe('validationScopes', () => {
  it('scopes by the project a domain declares and by the domain when it declares none', () => {
    const model = buildModel([
      { domain: { id: id('shop', 'shop'), project: { id: id('acme', 'acme') } } },
      { domain: { id: id('ops', 'ops'), project: { id: id('acme', 'acme') } } },
      { domain: { id: id('legacy', 'legacy') } },
    ] as never);
    expect(validationScopes(model)).toEqual([
      { projectNamespace: 'acme', projectSlug: 'acme', projectVersion: '1' },
      { domainNamespace: 'legacy', domainSlug: 'legacy', domainVersion: '1' },
      { namespace: 'legacy' },
    ]);
  });

  it('has no scopes without domains or contexts', () => {
    expect(validationScopes(buildModel([] as never))).toEqual([]);
  });

  it('adds no namespace scope for a context whose subdomain reaches a loaded domain with a project', () => {
    const model = buildModel([
      { domain: { id: id('shop', 'shop'), project: { id: id('acme', 'acme') } } },
      { subdomain: { id: id('shop', 'identity'), domain: { id: id('shop', 'shop') } } },
      { boundedContext: { id: id('accounts', 'accounts'), realizes: [{ id: id('shop', 'identity') }] } },
    ] as never);
    expect(validationScopes(model)).toEqual([{ projectNamespace: 'acme', projectSlug: 'acme', projectVersion: '1' }]);
  });

  it('adds a namespace scope for contexts that reach no domain, once per namespace', () => {
    const model = buildModel([
      { domain: { id: id('shop', 'shop'), project: { id: id('acme', 'acme') } } },
      { subdomain: { id: id('shop', 'identity'), domain: { id: id('shop', 'shop') } } },
      { boundedContext: { id: id('orphan', 'orphan') } },
      { boundedContext: { id: id('dangling', 'dangling'), realizes: [{ id: id('shop', 'missing') }] } },
      { boundedContext: { id: id('dangling', 'dangling-two') } },
      { boundedContext: { id: id('accounts', 'accounts'), realizes: [{ id: id('shop', 'identity') }] } },
    ] as never);
    expect(validationScopes(model)).toEqual([
      { projectNamespace: 'acme', projectSlug: 'acme', projectVersion: '1' },
      { namespace: 'orphan' },
      { namespace: 'dangling' },
    ]);
  });

  it('does not repeat a namespace scope already added for a project-less domain', () => {
    const model = buildModel([
      { domain: { id: id('legacy', 'legacy') } },
      { boundedContext: { id: id('legacy', 'legacy') } },
    ] as never);
    expect(validationScopes(model).filter((s) => s.namespace)).toEqual([{ namespace: 'legacy' }]);
  });
});

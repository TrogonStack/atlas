// Domain Chart invariants. The chart is a derived view of the
// problem-space knowledge graph (Decision #29): subdomains sit inside
// the band their `classification` names, bounded contexts hang under
// the subdomains they `realize`, and the "Unmapped" band only fires
// when there is something to flag.
import { describe, expect, it } from 'vitest';
import { buildDomainChart } from './domainchart';
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
});

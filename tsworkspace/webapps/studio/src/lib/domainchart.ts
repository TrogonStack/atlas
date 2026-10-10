// Core Domain Chart: the knowledge graph (Decision #29) as a glanceable
// investment map. Subdomains group into classification bands: CORE is
// where the best people belong, SUPPORTING is built cheaply, GENERIC is
// bought, and each subdomain shows the bounded contexts realizing it.
// Everything derives from Domain/Subdomain/realizes; no new data.
import type { Edge, Node } from '@xyflow/react';
import { classificationOf, type Entity, type Model, realizesOf } from './model';

export const CARD_W = 240;
export const CARD_H = 96;
export const CTX_W = 200;
export const CTX_H = 56;
const BAND_PAD = 40;
const BAND_HEADER = 48;
const COL_GAP = 48;
const BAND_GAP = 96;
const CTX_GAP = 24;
// Vertical gap between the subdomain card and the row of bounded
// contexts realizing it. Was 24 (inline); pulled into a constant and
// widened so the "realizes" arrows have room to breathe.
const SD_TO_CTX_GAP = 56;

export interface SubdomainCardData extends Record<string, unknown> {
  entity: Entity;
  classification?: string;
  selected?: boolean;
}

export interface ContextCardData extends Record<string, unknown> {
  entity: Entity;
  selected?: boolean;
}

export interface BandData extends Record<string, unknown> {
  label: string;
  hint: string;
  tone: 'core' | 'supporting' | 'generic' | 'unclassified';
}

const BANDS: { key: string; label: string; hint: string; tone: BandData['tone'] }[] = [
  { key: 'core', label: 'Core', hint: 'where you win or lose: best people, never outsourced', tone: 'core' },
  { key: 'supporting', label: 'Supporting', hint: 'necessary and bespoke: built cheaply', tone: 'supporting' },
  { key: 'generic', label: 'Generic', hint: 'a solved problem: buy, never build', tone: 'generic' },
  { key: 'unclassified', label: 'Unclassified', hint: 'classify it: this is a budget document', tone: 'unclassified' },
];

export function buildDomainChart(model: Model): { nodes: Node[]; edges: Edge[] } {
  const subdomains = model.entities.filter((e) => e.kind === 'subdomain');
  const contexts = model.entities.filter((e) => e.kind === 'boundedContext');
  const domains = model.entities.filter((e) => e.kind === 'domain');

  const realizedBySub = new Map<string, Entity[]>();
  const mapped = new Set<string>();
  for (const bc of contexts) {
    for (const rid of realizesOf(bc)) {
      const key = `${rid.namespace}/${rid.slug}`;
      const list = realizedBySub.get(key) ?? [];
      list.push(bc);
      realizedBySub.set(key, list);
      mapped.add(bc.key);
    }
  }

  const nodes: Node[] = [];
  const edges: Edge[] = [];
  // Pre-compute each band's width so the domain header spans the widest.
  const footprintOf = (sd: Entity) => {
    const n = (realizedBySub.get(`${sd.id.namespace}/${sd.id.slug}`) ?? []).length;
    const bcsW = n > 0 ? n * (CTX_W + CTX_GAP) - CTX_GAP : 0;
    return Math.max(CARD_W, bcsW);
  };
  const bandWidthForMembers = (mems: Entity[]) => {
    const fps = mems.map(footprintOf);
    return Math.max(fps.reduce((acc, w) => acc + w + COL_GAP, 0) - COL_GAP + BAND_PAD * 2, 3 * (CARD_W + COL_GAP));
  };
  const widestBand = BANDS.reduce(
    (w, band) => {
      const mems = subdomains.filter((sd) => (classificationOf(sd) ?? 'unclassified') === band.key);
      if (mems.length === 0) return w;
      return Math.max(w, bandWidthForMembers(mems));
    },
    4 * (CARD_W + COL_GAP),
  );

  let y = 0;

  for (const d of domains) {
    nodes.push({
      id: `dom:${d.key}`,
      type: 'domainHeader',
      position: { x: 0, y },
      style: { width: widestBand, height: 44 },
      data: { entity: d },
      draggable: false,
      zIndex: 2,
    });
    y += 44 + 48;
  }

  for (const band of BANDS) {
    const members = subdomains.filter((sd) => (classificationOf(sd) ?? 'unclassified') === band.key);
    if (members.length === 0) continue;
    members.sort((a, b) => a.title.localeCompare(b.title));

    // Each subdomain has a "footprint" wide enough to fan out its
    // realizing bounded contexts SIDE BY SIDE in one row, not stacked
    // in one column. Stacking made every BC->subdomain edge pass
    // through the BCs sitting between it and the subdomain, reading
    // as a chain when the relationships are independent in fact.
    const footprints = members.map(footprintOf);

    // Band height: room for subdomain card + one row of context cards.
    const hasAnyBcs = members.some((sd) => (realizedBySub.get(`${sd.id.namespace}/${sd.id.slug}`) ?? []).length > 0);
    const bandH = BAND_HEADER + BAND_PAD + CARD_H + (hasAnyBcs ? SD_TO_CTX_GAP + CTX_H : 0) + BAND_PAD;
    const bandW = bandWidthForMembers(members);

    nodes.push({
      id: `band:${band.key}`,
      type: 'classificationBand',
      position: { x: 0, y },
      style: { width: bandW, height: bandH },
      data: { label: band.label, hint: band.hint, tone: band.tone } satisfies BandData,
      draggable: false,
      selectable: false,
      zIndex: 0,
    });

    let cursorX = BAND_PAD;
    members.forEach((sd, i) => {
      const footprint = footprints[i];
      const sdX = cursorX + (footprint - CARD_W) / 2;
      const sdY = y + BAND_HEADER + BAND_PAD;
      nodes.push({
        id: `sd:${sd.key}`,
        type: 'subdomainCard',
        position: { x: sdX, y: sdY },
        style: { width: CARD_W, height: CARD_H },
        data: { entity: sd, classification: classificationOf(sd) } satisfies SubdomainCardData,
        draggable: false,
        zIndex: 2,
      });
      const realizers = realizedBySub.get(`${sd.id.namespace}/${sd.id.slug}`) ?? [];
      const rowW = realizers.length > 0 ? realizers.length * (CTX_W + CTX_GAP) - CTX_GAP : 0;
      const rowStart = cursorX + (footprint - rowW) / 2;
      realizers.forEach((bc, j) => {
        nodes.push({
          id: `ctx:${sd.key}:${bc.key}`,
          type: 'contextCard',
          position: { x: rowStart + j * (CTX_W + CTX_GAP), y: sdY + CARD_H + SD_TO_CTX_GAP },
          style: { width: CTX_W, height: CTX_H },
          data: { entity: bc } satisfies ContextCardData,
          draggable: false,
          zIndex: 2,
        });
        edges.push({
          id: `rel:${bc.key}->${sd.key}`,
          source: `ctx:${sd.key}:${bc.key}`,
          sourceHandle: 't',
          target: `sd:${sd.key}`,
          targetHandle: 'b',
          animated: false,
          style: { stroke: '#6366f1', strokeWidth: 1.6 },
          zIndex: 1,
          data: {
            relation: 'realizes',
            doc: '',
            metadata: [],
            source: bc,
            target: sd,
            slice: undefined,
          },
        });
      });
      cursorX += footprint + COL_GAP;
    });

    y += bandH + BAND_GAP;
  }

  // Contexts realizing nothing: unmapped territory, made visible on purpose.
  const unmapped = contexts.filter((bc) => !mapped.has(bc.key));
  if (unmapped.length > 0) {
    const unmappedW = Math.max(3 * (CARD_W + COL_GAP), unmapped.length * (CTX_W + COL_GAP) - COL_GAP + BAND_PAD * 2);
    nodes.push({
      id: 'band:unmapped',
      type: 'classificationBand',
      position: { x: 0, y },
      style: { width: unmappedW, height: BAND_HEADER + BAND_PAD * 2 + CTX_H },
      data: {
        label: 'Unmapped contexts',
        hint: 'realize a subdomain or explain why not: silence is not a strategy',
        tone: 'unclassified',
      } satisfies BandData,
      draggable: false,
      selectable: false,
      zIndex: 0,
    });
    unmapped.forEach((bc, i) => {
      nodes.push({
        id: `ctx:unmapped:${bc.key}`,
        type: 'contextCard',
        position: { x: BAND_PAD + i * (CTX_W + COL_GAP), y: y + BAND_HEADER + BAND_PAD },
        style: { width: CTX_W, height: CTX_H },
        data: { entity: bc } satisfies ContextCardData,
        draggable: false,
        zIndex: 2,
      });
    });
  }

  return { nodes, edges };
}

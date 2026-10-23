// Core Domain Chart: the knowledge graph (Decision #29) as a glanceable
// investment map. Subdomains group into classification bands: CORE is
// where the best people belong, SUPPORTING is built cheaply, GENERIC is
// bought, and each subdomain shows the bounded contexts realizing it.
// Everything derives from Domain/Subdomain/realizes; no new data.
import type { Edge, Node } from '@xyflow/react';
import type { ValidateProjectScope } from './api';
import type { Issue, Severity } from './issues';
import {
  classificationOf,
  domainOf,
  type Entity,
  type EntityId,
  type EntityKind,
  entityKey,
  type Model,
  realizesOf,
} from './model';

export const CARD_W = 240;
export const CARD_H = 96;
export const CTX_W = 240;
export const CTX_H = 68;
const BAND_PAD = 40;
const BAND_HEADER = 48;
const COL_GAP = 48;
const BAND_GAP = 96;
const CTX_GAP = 24;
// Vertical gap between the subdomain card and the row of bounded
// contexts realizing it. Was 24 (inline); pulled into a constant and
// widened so the "realizes" arrows have room to breathe.
const SD_TO_CTX_GAP = 56;

export const SPLIT_CANDIDATE_CODE = 'REALIZES_MIXED_CLASSIFICATIONS';

const CHART_SUBJECT_KINDS: ReadonlySet<EntityKind> = new Set(['boundedContext', 'domain', 'subdomain']);
const SEVERITY_ORDER: Record<Severity, number> = { error: 0, warning: 1, info: 2 };

export interface ChartFinding {
  severity: Severity;
  code: string;
  message: string;
}

export interface ChartMarker {
  /** The worst severity among the findings. */
  severity: Severity;
  /** Worst first. */
  findings: ChartFinding[];
}

/** Server findings per chart entity, keyed by `Entity.key`. */
export type ChartMarkers = ReadonlyMap<string, ChartMarker>;

export const NO_MARKERS: ChartMarkers = new Map();

/** Keeps the findings whose subject is a context, domain or subdomain and groups them by entity. */
export function chartMarkers(issues: readonly Issue[]): ChartMarkers {
  const grouped = new Map<string, ChartFinding[]>();
  for (const issue of issues) {
    if (!issue.subject || !CHART_SUBJECT_KINDS.has(issue.subject.kind)) continue;
    const key = entityKey(issue.subject.kind, issue.subject.id);
    const bucket = grouped.get(key) ?? [];
    bucket.push({ severity: issue.severity, code: issue.code, message: issue.message });
    grouped.set(key, bucket);
  }
  const markers = new Map<string, ChartMarker>();
  for (const [key, findings] of grouped) {
    findings.sort((a, b) => SEVERITY_ORDER[a.severity] - SEVERITY_ORDER[b.severity] || a.code.localeCompare(b.code));
    markers.set(key, { severity: findings[0].severity, findings });
  }
  return markers;
}

export function markerTooltip(marker: ChartMarker): string {
  return marker.findings.map((f) => `${f.code}: ${f.message}`).join('\n');
}

/**
 * The validation scopes that cover the chart. Each domain is covered by the
 * Project it declares, otherwise by itself. A namespace scope is added for a
 * problem-space namespace whose Domain declares no Project, and for each
 * context whose realized subdomains reach no Domain in the model, so that
 * strategic findings on them are still reported. Each scope appears once.
 */
export function validationScopes(model: Model): ValidateProjectScope[] {
  const covering = new Map<string, ValidateProjectScope>();
  const namespaces = new Map<string, ValidateProjectScope>();
  const domainKeys = new Set<string>();
  for (const domain of model.entities) {
    if (domain.kind !== 'domain') continue;
    domainKeys.add(`${domain.id.namespace}/${domain.id.slug}`);
    const project = (
      domain.raw.project as { id?: { namespace?: string; slug?: string; version?: unknown } } | undefined
    )?.id;
    if (project?.namespace && project.slug) {
      covering.set(`project:${project.namespace}/${project.slug}`, {
        projectNamespace: project.namespace,
        projectSlug: project.slug,
        projectVersion: project.version ? String(project.version) : undefined,
      });
    } else {
      covering.set(`domain:${domain.id.namespace}/${domain.id.slug}`, {
        domainNamespace: domain.id.namespace,
        domainSlug: domain.id.slug,
        domainVersion: domain.id.version,
      });
      namespaces.set(domain.id.namespace, { namespace: domain.id.namespace });
    }
  }
  const subdomainDomain = new Map<string, EntityId | undefined>();
  for (const sd of model.entities) {
    if (sd.kind === 'subdomain') subdomainDomain.set(`${sd.id.namespace}/${sd.id.slug}`, domainOf(sd));
  }
  for (const bc of model.entities) {
    if (bc.kind !== 'boundedContext') continue;
    const reachesDomain = realizesOf(bc).some((rid) => {
      const parent = subdomainDomain.get(`${rid.namespace}/${rid.slug}`);
      return parent ? domainKeys.has(`${parent.namespace}/${parent.slug}`) : false;
    });
    if (!reachesDomain) namespaces.set(bc.id.namespace, { namespace: bc.id.namespace });
  }
  return [...covering.values(), ...namespaces.values()];
}

export interface SubdomainCardData extends Record<string, unknown> {
  entity: Entity;
  classification?: string;
  marker?: ChartMarker;
  selected?: boolean;
}

export interface SharedSubdomain {
  entity: Entity;
  title: string;
  classification: string;
}

export interface ContextCardData extends Record<string, unknown> {
  entity: Entity;
  classification?: string;
  alsoRealizes: SharedSubdomain[];
  /** Set only when the server reports REALIZES_MIXED_CLASSIFICATIONS for this context. */
  splitCandidate?: string;
  marker?: ChartMarker;
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

export function splitCandidateText(bands: string[]): string {
  const names = bands.length > 1 ? `${bands.slice(0, -1).join(', ')} and ${bands[bands.length - 1]}` : bands.join('');
  return `split candidate: ${names}`;
}

export function buildDomainChart(model: Model, markers: ChartMarkers = NO_MARKERS): { nodes: Node[]; edges: Edge[] } {
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

  const subdomainByKey = new Map(subdomains.map((sd) => [`${sd.id.namespace}/${sd.id.slug}`, sd]));
  const bandOf = (sd: Entity) => classificationOf(sd) ?? 'unclassified';
  const contextCardData = (bc: Entity, home?: Entity): ContextCardData => {
    const alsoRealizes: SharedSubdomain[] = [];
    for (const rid of realizesOf(bc)) {
      const other = subdomainByKey.get(`${rid.namespace}/${rid.slug}`);
      if (!home || !other || other.key === home.key) continue;
      alsoRealizes.push({ entity: other, title: other.title, classification: bandOf(other) });
    }
    const marker = markers.get(bc.key);
    const touched = new Set([...(home ? [bandOf(home)] : []), ...alsoRealizes.map((o) => o.classification)]);
    const bands = BANDS.map((b) => b.key).filter((k) => touched.has(k));
    const reported = marker?.findings.some((f) => f.code === SPLIT_CANDIDATE_CODE) ?? false;
    return {
      entity: bc,
      classification: home ? bandOf(home) : undefined,
      alsoRealizes,
      splitCandidate: reported && bands.length > 1 ? splitCandidateText(bands) : undefined,
      marker,
    };
  };

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
      data: { entity: d, marker: markers.get(d.key) },
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
        data: {
          entity: sd,
          classification: classificationOf(sd),
          marker: markers.get(sd.key),
        } satisfies SubdomainCardData,
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
          data: contextCardData(bc, sd),
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
        data: contextCardData(bc),
        draggable: false,
        zIndex: 2,
      });
    });
  }

  return { nodes, edges };
}

// Sitemap mode of the UI view: the app surface as a navigable map. Nodes
// are OCCURRENCES of UI surfaces (a GUI screen, a voice prompt, a CLI),
// edges are derived navigations: from one surface the persona issued a
// command, the system did whatever it did, and the journey landed on the
// next surface. Edges collapse the in-between (the Journey mode shows it)
// and are labeled by the initiating command.
//
// Decision #24 (TIME ONLY POINTS FORWARD) binds this derived graph too.
// Collapsing every moment of a surface into one type-level node is where
// backward edges sneak in: two surfaces "pointing at each other" claim to
// be each other's past. So the collapse goes only as far as the graph
// stays forward: a landing reuses the earliest existing occurrence of its
// surface that lies AFTER where we are, and the first arrow that would
// point backward instead mints a NEW occurrence (Sign-up #1 → Settings #1
// → Sign-up #2). Multiple occurrences of one type are grouping, not
// duplication. Everything derives from the storyboards; no new data.
import type { Edge, Node } from '@xyflow/react';
import type { BoardEdgeData } from './layout';
import { asId, type Entity, type Model, type SliceView, slugKey, uiTransitionsOf, versionOf } from './model';

export const SITE_W = 232;
export const SITE_H = 108;
const COL_GAP = 130;
const ROW_GAP = 56;

export interface SitemapNodeData extends Record<string, unknown> {
  entity: Entity;
  ui: Entity;
  persona?: Entity;
  displayCount: number;
  actionCount: number;
  // "#2/4": which occurrence of this surface the node is: same type as
  // its twins, not the same thing.
  instance?: string;
  selected?: boolean;
}

interface Nav {
  from: Occ;
  to: Occ;
  via?: Entity;
  // Declared pure navigation (UI.transitions): no command, no event;
  // labeled by its doc instead of a carrier.
  declared?: string;
}

interface Occ {
  id: number;
  ui: Entity;
  // Insert-time layer: every edge strictly increases it, which is what
  // guarantees the graph is forward-only (acyclic).
  layer: number;
}

export function buildSitemap(
  model: Model,
  onDroppedBackwardEdge?: (from: string, to: string) => void,
): { nodes: Node[]; edges: Edge[] } {
  const bySlug = new Map<string, Entity>();
  for (const e of model.entities) {
    const k = slugKey(e.kind, e.id);
    const cur = bySlug.get(k);
    if (!cur || versionOf(e.id) > versionOf(cur.id)) bySlug.set(k, e);
  }
  const resolve = (kind: Entity['kind'], ref: { id: { namespace: string; slug: string } } | undefined) =>
    ref ? bySlug.get(slugKey(kind, ref.id)) : undefined;
  const sliceBySlug = new Map(model.slices.map((s) => [`${s.entity.id.namespace}/${s.entity.id.slug}`, s]));

  const occs: Occ[] = [];
  const occsByType = new Map<string, Occ[]>();
  const navs = new Map<string, Nav>();
  const personaOf = new Map<string, Entity>();
  const displays = new Map<string, Set<string>>();
  const actions = new Map<string, Set<string>>();

  const mint = (ui: Entity, layer: number): Occ => {
    const occ: Occ = { id: occs.length, ui, layer };
    occs.push(occ);
    const list = occsByType.get(ui.key) ?? [];
    list.push(occ);
    occsByType.set(ui.key, list);
    return occ;
  };

  const walk = (slices: SliceView[]) => {
    let current: Occ | undefined;
    let carrier: Entity | undefined;
    const land = (ui: Entity) => {
      const candidates = occsByType.get(ui.key) ?? [];
      let target: Occ | undefined;
      if (!current) {
        // A journey can start on any occurrence; the earliest keeps the
        // map compact.
        target = candidates[0] ?? mint(ui, 0);
      } else if (current.ui.key === ui.key && !carrier) {
        // Nothing happened in between: still the same moment.
        target = current;
      } else {
        // Forward only: reuse the earliest occurrence strictly after
        // where we are, else this landing is a new moment of the type.
        target = candidates.filter((o) => o.layer > (current as Occ).layer).sort((a, b) => a.layer - b.layer)[0];
        if (!target) target = mint(ui, current.layer + 1);
        const k = `${current.id}->${target.id}@${carrier?.key ?? ''}`;
        if (!navs.has(k)) navs.set(k, { from: current, to: target, via: carrier });
      }
      current = target;
      carrier = undefined;
    };
    for (const s of slices) {
      if (s.kind === 'commandSlice') {
        const ui = resolve('ui', s.ui);
        const command = resolve('command', s.command);
        const persona = s.persona ? resolve('persona', s.persona) : undefined;
        if (ui) {
          land(ui);
          if (persona && !personaOf.has(ui.key)) personaOf.set(ui.key, persona);
          if (command) {
            const set = actions.get(ui.key) ?? new Set<string>();
            set.add(command.key);
            actions.set(ui.key, set);
          }
        }
        // The command initiates whatever comes next; automation chains in
        // between keep its name on the edge.
        if (command) carrier = command;
      } else if (s.kind === 'uiSlice') {
        const ui = resolve('ui', s.ui);
        const persona = s.persona ? resolve('persona', s.persona) : undefined;
        if (ui) {
          land(ui);
          if (persona && !personaOf.has(ui.key)) personaOf.set(ui.key, persona);
          const set = displays.get(ui.key) ?? new Set<string>();
          for (const r of s.readModels) {
            const rm = resolve('readModel', r);
            if (rm) set.add(rm.key);
          }
          displays.set(ui.key, set);
        }
      }
      // Automation slices keep the carrier: the initiating command labels
      // the whole hop, however many machines ran in between.
    }
  };

  for (const sb of model.storyboards) {
    const slices: SliceView[] = [];
    for (const ref of Array.isArray(sb.raw.slices) ? sb.raw.slices : []) {
      const id = asId((ref as Record<string, unknown>)?.id);
      const slice = sliceBySlug.get(`${id.namespace}/${id.slug}`);
      if (slice) slices.push(slice);
    }
    walk(slices);
  }

  // The sitemap is the full surface inventory: surfaces no storyboard
  // touches still exist. Use latest-by-slug so an older version listed
  // later is not minted as a separate "untouched" surface.
  for (const e of bySlug.values()) if (e.kind === 'ui' && !occsByType.has(e.key)) mint(e, 0);

  // Declared pure navigation (UI.transitions) joins the map after the
  // walks: from the earliest occurrence of the source, reuse the earliest
  // occurrence of the target strictly after it, else mint, the same
  // forward-only rule. Cycles in the declared data unroll here.
  for (const e of bySlug.values()) {
    for (const t of uiTransitionsOf(e)) {
      const target = bySlug.get(slugKey('ui', t.to));
      if (!target) continue;
      const from = (occsByType.get(e.key) ?? [])[0];
      if (!from) continue;
      let to = (occsByType.get(target.key) ?? [])
        .filter((o) => o.layer > from.layer)
        .sort((a, b) => a.layer - b.layer)[0];
      if (!to) to = mint(target, from.layer + 1);
      const k = `decl:${from.id}->${to.id}`;
      if (!navs.has(k)) navs.set(k, { from, to, declared: t.doc || 'navigates' });
    }
  }

  const navList = [...navs.values()];

  // Longest-path layering over the (guaranteed acyclic) occurrence graph,
  // so each occurrence sits after everything that leads to it.
  const out = new Map<number, { to: number }[]>();
  const indeg = new Map<number, number>();
  for (const o of occs) {
    out.set(o.id, []);
    indeg.set(o.id, 0);
  }
  for (const nav of navList) {
    out.get(nav.from.id)?.push({ to: nav.to.id });
    indeg.set(nav.to.id, (indeg.get(nav.to.id) ?? 0) + 1);
  }
  const depth = new Map<number, number>();
  const queue = occs.filter((o) => (indeg.get(o.id) ?? 0) === 0).map((o) => o.id);
  for (const id of queue) depth.set(id, 0);
  // Defense-in-depth iteration cap. The forward filter above is supposed
  // to keep the navs graph acyclic, but a future relaxation (e.g. allowing
  // backward edges for "undo" navs) would otherwise hang the layout.
  const MAX_ITER = occs.length * 16 + 1024;
  let iter = 0;
  while (queue.length > 0) {
    if (iter++ > MAX_ITER) break;
    const id = queue.shift() as number;
    for (const { to } of out.get(id) ?? []) {
      const d = (depth.get(id) ?? 0) + 1;
      if (d > (depth.get(to) ?? 0)) depth.set(to, d);
      indeg.set(to, (indeg.get(to) ?? 0) - 1);
      if ((indeg.get(to) ?? 0) === 0) queue.push(to);
    }
  }

  const columns = new Map<number, Occ[]>();
  for (const o of occs) {
    const d = depth.get(o.id) ?? 0;
    const col = columns.get(d) ?? [];
    col.push(o);
    columns.set(d, col);
  }

  const nodes: Node[] = [];
  for (const [d, col] of [...columns.entries()].sort((a, b) => a[0] - b[0])) {
    col.sort((a, b) => a.ui.title.localeCompare(b.ui.title) || a.id - b.id);
    col.forEach((o, i) => {
      const twins = occsByType.get(o.ui.key) ?? [];
      const ord = twins.indexOf(o) + 1;
      nodes.push({
        id: `site:${o.id}:${o.ui.key}`,
        type: 'sitemapScreen',
        position: { x: d * (SITE_W + COL_GAP), y: i * (SITE_H + ROW_GAP) },
        style: { width: SITE_W, height: SITE_H },
        data: {
          entity: o.ui,
          ui: o.ui,
          persona: personaOf.get(o.ui.key),
          displayCount: displays.get(o.ui.key)?.size ?? 0,
          actionCount: actions.get(o.ui.key)?.size ?? 0,
          instance: twins.length > 1 ? `#${ord}/${twins.length}` : undefined,
        } satisfies SitemapNodeData,
        draggable: false,
        zIndex: 2,
      });
    });
  }

  // Decision #24 enforcement, not just intent: a navigation that does not
  // move time forward must never render. The minting rule above makes this
  // unreachable; if a future change breaks it, the edge is dropped and the
  // caller is notified via the optional callback (defaults to a no-op).
  const forward = navList.filter((nav) => {
    const ok = (depth.get(nav.from.id) ?? 0) < (depth.get(nav.to.id) ?? 0);
    if (!ok) onDroppedBackwardEdge?.(nav.from.ui.title, nav.to.ui.title);
    return ok;
  });

  const edges: Edge[] = forward.map((nav) => {
    // A hop between surfaces owned by different personas is not a
    // navigation: nobody clicks from the customer's cart into the
    // admin's console. It is a handoff: one persona's command surfaces on
    // another persona's screen.
    const fromPersona = personaOf.get(nav.from.ui.key);
    const toPersona = personaOf.get(nav.to.ui.key);
    const handoff = !nav.declared && Boolean(fromPersona && toPersona && fromPersona.key !== toPersona.key);
    if (nav.declared) {
      return {
        id: `site:decl:${nav.from.id}->${nav.to.id}`,
        source: `site:${nav.from.id}:${nav.from.ui.key}`,
        sourceHandle: 'r',
        target: `site:${nav.to.id}:${nav.to.ui.key}`,
        targetHandle: 'l',
        animated: false,
        style: { stroke: '#a1a1aa', strokeWidth: 1.6 },
        zIndex: 3,
        data: {
          relation: `declared navigation: ${nav.declared}`,
          doc: nav.declared,
          metadata: [],
          source: nav.from.ui,
          target: nav.to.ui,
          slice: undefined,
        } satisfies BoardEdgeData,
      };
    }
    return {
      id: `site:${nav.from.id}->${nav.to.id}@${nav.via?.key ?? ''}`,
      source: `site:${nav.from.id}:${nav.from.ui.key}`,
      sourceHandle: 'r',
      target: `site:${nav.to.id}:${nav.to.ui.key}`,
      targetHandle: 'l',
      animated: !handoff,
      style: handoff
        ? { stroke: '#f59e0b', strokeWidth: 1.8, strokeDasharray: '6 4' }
        : { stroke: '#0ea5e9', strokeWidth: 1.8 },
      zIndex: 3,
      data: {
        relation: handoff
          ? `hands off to ${toPersona?.title}${nav.via ? ` via ${nav.via.title}` : ''}`
          : nav.via
            ? `navigates via ${nav.via.title}`
            : 'navigates',
        doc: '',
        metadata: [],
        source: nav.from.ui,
        target: nav.to.ui,
        slice: nav.via,
      } satisfies BoardEdgeData,
    };
  });

  return { nodes, edges };
}

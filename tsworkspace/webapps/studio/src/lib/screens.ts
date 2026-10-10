// UI view: the model for designers, storyboards rendered as user journeys
// of framed UI moments. A UI is any interaction surface (a GUI screen, a
// voice prompt, a CLI), and each frame is one MOMENT of it (a new frame
// whenever the surface changes or the system acted in between; time only
// points forward, a surface presented again is a new occurrence): the
// persona using it, the data it displays (read models rendered into it),
// and the actions it offers (commands issued from it). Whatever happens
// without a screen (automations, projections, emitted events) compresses
// into small "system" steps between frames. Everything derives from the
// slices; no new data.
import type { Edge, Node } from '@xyflow/react';
import type { Glyph } from './glyphs';
import type { BoardEdgeData } from './layout';
import {
  asId,
  type Entity,
  instanceUidOf,
  type Model,
  orderedSlices,
  type SliceView,
  slugKey,
  versionOf,
} from './model';

export const FRAME_W = 248;
export const FRAME_H = 300;
export const SYS_H = 110;
const STEP = FRAME_W + 64;
const SB_H = 30;
const ROW_H = FRAME_H + SB_H + 110;

export interface ScreenFrameData extends Record<string, unknown> {
  ui: Entity;
  persona?: Entity;
  displays: Entity[];
  actions: Entity[];
  // "#2/4": which moment of this screen the frame is.
  instance?: string;
  selected?: boolean;
}

// A step's line carries its own glyph so the renderer draws a vector icon
// instead of the text prefixing itself with an emoji.
export interface SystemStepLine {
  text: string;
  glyph?: Glyph;
}

export interface SystemStepData extends Record<string, unknown> {
  entity: Entity;
  lines: SystemStepLine[];
  selected?: boolean;
}

type Item =
  | { kind: 'frame'; ui: Entity; persona?: Entity; displays: Entity[]; actions: Entity[]; moment?: Entity }
  | { kind: 'sys'; entity: Entity; lines: SystemStepLine[]; moment?: Entity };

export function buildScreens(model: Model): { nodes: Node[]; edges: Edge[] } {
  const bySlug = new Map<string, Entity>();
  for (const e of model.entities) {
    const k = slugKey(e.kind, e.id);
    const cur = bySlug.get(k);
    if (!cur || versionOf(e.id) > versionOf(cur.id)) bySlug.set(k, e);
  }
  const resolve = (kind: Entity['kind'], ref: { id: { namespace: string; slug: string } } | undefined) =>
    ref ? bySlug.get(slugKey(kind, ref.id)) : undefined;
  const sliceBySlug = new Map(model.slices.map((s) => [`${s.entity.id.namespace}/${s.entity.id.slug}`, s]));
  const claimed = new Set<string>();

  const rows: { storyboard?: Entity; items: Item[] }[] = [];

  const buildRow = (storyboard: Entity | undefined, slices: SliceView[]) => {
    const items: Item[] = [];
    let frame: Extract<Item, { kind: 'frame' }> | undefined;
    const newFrame = (ui: Entity, moment?: Entity, persona?: Entity) => {
      frame = { kind: 'frame', ui, persona, displays: [], actions: [], moment };
      items.push(frame);
      return frame;
    };
    const sys = (entity: Entity, line: SystemStepLine, moment?: Entity) => {
      const last = items[items.length - 1];
      if (last?.kind === 'sys') last.lines.push(line);
      else items.push({ kind: 'sys', entity, lines: [line], moment });
      frame = undefined; // the world changed: the next screen is a new moment
    };
    for (const s of slices) {
      if (s.kind === 'commandSlice') {
        const ui = resolve('ui', s.ui);
        const command = resolve('command', s.command);
        const persona = s.persona ? resolve('persona', s.persona) : undefined;
        if (ui && command) {
          const f = frame && frame.ui.key === ui.key ? frame : newFrame(ui, s.entity, persona);
          if (persona && !f.persona) f.persona = persona;
          if (!f.actions.some((a) => a.key === command.key)) f.actions.push(command);
        }
        const emitted = s.events
          .map((e) => resolve('event', e))
          .filter((e): e is Entity => Boolean(e))
          .map((e) => e.title)
          .join(', ');
        if (command) sys(command, { text: ui ? `→ ${emitted}` : `${command.title} → ${emitted}` }, s.entity);
      } else if (s.kind === 'readModelSlice') {
        const rm = resolve('readModel', s.readModel);
        if (rm) sys(rm, { text: `updates ${rm.title}` }, s.entity);
      } else if (s.kind === 'uiSlice') {
        const ui = resolve('ui', s.ui);
        const persona = s.persona ? resolve('persona', s.persona) : undefined;
        if (ui) {
          const f = frame && frame.ui.key === ui.key ? frame : newFrame(ui, s.entity, persona);
          if (persona && !f.persona) f.persona = persona;
          for (const r of s.readModels) {
            const rm = resolve('readModel', r);
            if (rm && !f.displays.some((d) => d.key === rm.key)) f.displays.push(rm);
          }
        }
      } else {
        const proc = resolve('processor', s.processor);
        if (proc) sys(proc, { text: proc.title, glyph: 'automation' }, s.entity);
      }
    }
    if (items.length > 0) rows.push({ storyboard, items });
  };

  for (const sb of model.storyboards) {
    const slices: SliceView[] = [];
    for (const ref of Array.isArray(sb.raw.slices) ? sb.raw.slices : []) {
      const id = asId((ref as Record<string, unknown>)?.id);
      const k = `${id.namespace}/${id.slug}`;
      const slice = sliceBySlug.get(k);
      if (!slice || claimed.has(k)) continue;
      claimed.add(k);
      slices.push(slice);
    }
    buildRow(sb, slices);
  }
  const leftovers = orderedSlices(model).filter((s) => !claimed.has(`${s.entity.id.namespace}/${s.entity.id.slug}`));
  if (leftovers.length > 0) buildRow(undefined, leftovers);

  // Frame ordinals: the same UI presented again is a new moment.
  const frameTotals = new Map<string, number>();
  for (const row of rows)
    for (const item of row.items)
      if (item.kind === 'frame') frameTotals.set(item.ui.key, (frameTotals.get(item.ui.key) ?? 0) + 1);
  const frameSeen = new Map<string, number>();

  const nodes: Node[] = [];
  const edges: Edge[] = [];
  rows.forEach((row, r) => {
    const baseY = r * ROW_H;
    if (row.storyboard) {
      nodes.push({
        id: `scr-sb:${row.storyboard.key}`,
        type: 'seqStoryboardHeader',
        position: { x: 0, y: baseY },
        style: { width: (row.items.length - 1) * STEP + FRAME_W, height: SB_H },
        data: { entity: row.storyboard },
        draggable: false,
        zIndex: 1,
      });
    }
    row.items.forEach((item, i) => {
      const id = `scr-${r}-${i}`;
      if (item.kind === 'frame') {
        nodes.push({
          id,
          type: 'screenFrame',
          position: { x: i * STEP, y: baseY + SB_H + 12 },
          style: { width: FRAME_W, height: FRAME_H },
          data: {
            entity: item.ui, // selection/focus key
            ui: item.ui,
            persona: item.persona,
            displays: item.displays,
            actions: item.actions,
            moment: item.moment,
            instanceUid: instanceUidOf(item.ui, item.moment),
            instance: (() => {
              const total = frameTotals.get(item.ui.key) ?? 0;
              if (total < 2) return undefined;
              const n = (frameSeen.get(item.ui.key) ?? 0) + 1;
              frameSeen.set(item.ui.key, n);
              return `#${n}/${total}`;
            })(),
          },
          draggable: false,
          zIndex: 2,
        });
      } else {
        nodes.push({
          id,
          type: 'systemStep',
          position: { x: i * STEP, y: baseY + SB_H + 12 + (FRAME_H - SYS_H) / 2 },
          style: { width: FRAME_W, height: SYS_H },
          data: {
            entity: item.entity,
            lines: item.lines,
            moment: item.moment,
            instanceUid: instanceUidOf(item.entity, item.moment),
          },
          draggable: false,
          zIndex: 2,
        });
      }
      if (i > 0) {
        const prev = row.items[i - 1];
        edges.push({
          id: `${id}-next`,
          source: `scr-${r}-${i - 1}`,
          sourceHandle: 'r',
          target: id,
          targetHandle: 'l',
          animated: true,
          style: { stroke: '#a1a1aa', strokeWidth: 1.8 },
          zIndex: 3,
          data: {
            relation: 'then',
            doc: '',
            metadata: [],
            source: prev.kind === 'frame' ? prev.ui : prev.entity,
            target: item.kind === 'frame' ? item.ui : item.entity,
            slice: undefined,
          } satisfies BoardEdgeData,
        });
      }
    });
  });

  return { nodes, edges };
}

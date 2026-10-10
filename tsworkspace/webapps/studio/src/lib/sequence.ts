// Sequence view: the model flattened into forward temporal chains; no
// lanes, no zigzag. Each occurrence points into the next (ui -> command ->
// event -> read model -> processor -> ...). This relaxes the EM edge grammar
// on purpose: plain arrows mean "happens next", but an arrow between two
// cards of the same kind reads as a data dependency the model does not
// have, so three rules keep the chain from drawing one:
//   1. Same-moment peers STACK vertically in one column (consecutive
//      projections, co-emitted events, an automation's source read models)
//      and the previous moment fans out into the stack.
//   2. A card already on the board is not drawn twice one column later:
//      an automation whose sources are all projected to its left observes
//      those occurrences, and a command slice handling a command an
//      automation just emitted continues from that card.
//   3. A moment that names what it reads (`Slot.observes`) is entered only
//      from those cards: the rest of a projection stack are its siblings,
//      not its inputs.
//   4. A screen is a dead end: a UiSlice hands read models to a person and
//      nothing in the model flows back out of it. Its column is transparent
//      (`Slot.transparent`), so the moment after it continues from the state
//      it displayed rather than from the screen.
//
// Forks (derived from Swimlane.transitions, the stream decides) branch the
// chain: each continuation chunk starts on its own row to the right of its
// parent, the source chunk centered between its branches (balanced tree),
// connected by a dashed edge labeled with the guard condition. The edge
// leaves from the read-model occurrence the branch's entry observes: you
// observe state, therefore you can act. Time only moves forward: cycles end
// in a forward loop stub, never a backward edge.
import type { Edge, Node } from '@xyflow/react';
import { type BoardEdgeData, NODE_H, NODE_W } from './layout';
import {
  asId,
  type Entity,
  instanceUidOf,
  type Model,
  orderedSlices,
  type SliceView,
  slugKey,
  storyboardEntryReadModel,
  trackingOf,
} from './model';

const GAP = 56;
const STEP = NODE_W + GAP;
const SB_Y = 0;
const SB_H = 30;
const SLICE_Y = 40;
const SLICE_H = 44;
const ITEM_Y = 100;
// Vertical pitch between stacked projections (slice header + card + air).
const PITCH = NODE_H + SLICE_H + 36;

export interface SeqStickyData extends Record<string, unknown> {
  entity: Entity;
  selected: boolean;
  // "#2/4": position among this type's occurrences in this view.
  instance?: string;
  // The slice defining this moment: the instance identity.
  moment?: Entity;
  // uuidv5(type uid / defining-slice uid): the machine id of THIS moment.
  instanceUid?: string;
  // Branch review status (Phase 3: Studio), see `StickyData.branchStatus`
  // in layout.ts for the same convention, applied here post-hoc too.
  branchStatus?: string;
}

interface SlotMember {
  entity: Entity;
  // Read-model members carry their slice for a per-card header; command and
  // automation slices keep band headers via `segments`.
  slice?: SliceView;
  // The slice defining this moment: the instance identity for every member.
  moment?: SliceView;
}

// One moment on the time axis: the cards that happen together, plus what
// they read from the moment before.
interface Slot {
  members: SlotMember[];
  // Entity keys in the PREVIOUS slot this moment actually observes. Set by
  // the consumers (automations and UI slices) which read named state
  // rather than everything that happens to share the previous moment.
  // Unset means "continue from all of it".
  observes?: string[];
  // A moment nothing flows out of: the chain reads straight through it.
  transparent?: boolean;
}

interface Chunk {
  storyboard?: Entity;
  slots: Slot[];
  segments: { slice: SliceView; start: number; end: number }[];
  offset: number;
  row: number;
}

export function buildSequence(model: Model): { nodes: Node[]; edges: Edge[] } {
  const bySlug = new Map(model.entities.map((e) => [slugKey(e.kind, e.id), e]));
  const resolve = (kind: Entity['kind'], ref: { id: { namespace: string; slug: string } } | undefined) =>
    ref ? bySlug.get(slugKey(kind, ref.id)) : undefined;

  const sliceBySlug = new Map(model.slices.map((s) => [`${s.entity.id.namespace}/${s.entity.id.slug}`, s]));
  const claimed = new Set<string>();

  const buildChunk = (storyboard: Entity | undefined, slices: SliceView[]): Chunk => {
    const slots: Slot[] = [];
    const segments: Chunk['segments'] = [];
    const holds = (slot: Slot | undefined, entity: Entity) =>
      Boolean(slot?.members.some((m) => m.entity.key === entity.key));
    // The run of projections still open to our left: consecutive read models
    // are one moment, so they stack there instead of chaining into a column
    // each. Cleared by any slice that is not a projection.
    let projection: Slot | undefined;
    // The last moment the chain continues from. A screen is a dead end, so
    // it never becomes one.
    let chainTail: Slot | undefined;
    for (const s of slices) {
      // The moment immediately to our left: what this slice may continue
      // from instead of re-drawing a card that is already there.
      const carried = chainTail;
      if (s.kind === 'readModelSlice') {
        const rm = resolve('readModel', s.readModel);
        if (rm) {
          const member: SlotMember = { entity: rm, slice: s, moment: s };
          if (projection) projection.members.push(member);
          else {
            projection = { members: [member] };
            slots.push(projection);
            chainTail = projection;
          }
        }
        continue;
      }
      projection = undefined;
      const start = slots.length;
      if (s.kind === 'commandSlice') {
        const ui = resolve('ui', s.ui);
        const command = resolve('command', s.command);
        // An automation to our left already issued this command: handling it
        // is the same moment, not the next one. Re-drawing the card would
        // point the command at a copy of itself. A slice with its own UI is
        // a human issuing it, so that occurrence stands on its own.
        const alreadyIssued = Boolean(command && !ui && holds(carried, command));
        if (ui) slots.push({ members: [{ entity: ui, moment: s }] });
        if (command && !alreadyIssued) slots.push({ members: [{ entity: command, moment: s }] });
        // Co-emitted events share one moment: stack in a single slot so the
        // command fans out (no event→event chain across successive columns).
        const events: SlotMember[] = [];
        for (const ev of s.events) {
          const event = resolve('event', ev);
          if (event) events.push({ entity: event, moment: s });
        }
        if (events.length > 0) slots.push({ members: events });
      } else {
        // The two consumers: an automation reads source read models into a
        // processor that issues a command, a UI slice reads them onto a
        // screen. The sources are read together, so they share one column
        // instead of chaining into each other; when the projections to our
        // left already show every one of them, the consumer observes those
        // occurrences rather than duplicating the cards a column later.
        const sources: SlotMember[] = [];
        for (const rm of s.readModels) {
          const entity = resolve('readModel', rm);
          if (entity) sources.push({ entity, moment: s });
        }
        const observes = sources.length > 0 ? sources.map((m) => m.entity.key) : undefined;
        const alreadyProjected = sources.length > 0 && sources.every((m) => holds(carried, m.entity));
        if (sources.length > 0 && !alreadyProjected) slots.push({ members: sources });
        if (s.kind === 'uiSlice') {
          const ui = resolve('ui', s.ui);
          if (ui) slots.push({ members: [{ entity: ui, moment: s }], observes, transparent: true });
        } else {
          const proc = resolve('processor', s.processor);
          const command = resolve('command', s.command);
          if (proc) slots.push({ members: [{ entity: proc, moment: s }], observes });
          if (command) slots.push({ members: [{ entity: command, moment: s }], observes: proc ? undefined : observes });
        }
      }
      if (slots.length > start) {
        segments.push({ slice: s, start, end: slots.length - 1 });
        // A screen is a dead end: the chain continues from the state it
        // displayed, so the moment after a UI slice skips over it.
        if (s.kind !== 'uiSlice') chainTail = slots[slots.length - 1];
      }
    }
    return { storyboard, slots, segments, offset: 0, row: 0 };
  };

  const chunks: Chunk[] = [];
  const chunkBySb = new Map<string, Chunk>();
  for (const sb of model.storyboards) {
    const refs = Array.isArray(sb.raw.slices) ? sb.raw.slices : [];
    const slices: SliceView[] = [];
    for (const ref of refs) {
      const id = asId((ref as Record<string, unknown>)?.id);
      const k = `${id.namespace}/${id.slug}`;
      const slice = sliceBySlug.get(k);
      if (!slice || claimed.has(k)) continue;
      claimed.add(k);
      slices.push(slice);
    }
    if (slices.length === 0) continue;
    const chunk = buildChunk(sb, slices);
    chunks.push(chunk);
    chunkBySb.set(sb.key, chunk);
  }
  const leftovers = orderedSlices(model).filter((s) => !claimed.has(`${s.entity.id.namespace}/${s.entity.id.slug}`));
  if (leftovers.length > 0) chunks.push(buildChunk(undefined, leftovers));

  // Row height adapts to the tallest projection stack in the model.
  const maxStack = Math.max(1, ...chunks.flatMap((c) => c.slots.map((s) => s.members.length)));
  const rowH = ITEM_Y + maxStack * PITCH + 64;

  // ---- placement -----------------------------------------------------------
  const children = new Map<string, { chunk: Chunk; doc: string }[]>();
  const isTarget = new Set<string>();
  for (const c of model.continuations) {
    const from = chunkBySb.get(c.from.key);
    const to = chunkBySb.get(c.to.key);
    if (!from?.storyboard || !to?.storyboard) continue;
    const list = children.get(from.storyboard.key) ?? [];
    list.push({ chunk: to, doc: c.doc });
    children.set(from.storyboard.key, list);
    isTarget.add(to.storyboard.key);
  }

  // Tidy-tree placement: a fork's branches stack into a band of rows and
  // the source chunk sits at the band's vertical CENTER (fractional rows),
  // so forks read as a balanced tree instead of hugging the top branch.
  // Returns the band height (in rows) this subtree occupies.
  const placed = new Set<Chunk>();
  const placementOrder = new Map<Chunk, number>();
  // Maintain a running max of (offset + slots) across every placed chunk
  // so root-cursor advancement is O(1) instead of O(n) per root (the
  // previous `Math.max(...[...placed].map(...))` was O(n²) overall and
  // could `RangeError` past ~100k chunks).
  let maxOffsetEnd = 0;
  const place = (chunk: Chunk, offset: number, rowStart: number): number => {
    if (placed.has(chunk)) return 0;
    placed.add(chunk);
    placementOrder.set(chunk, placementOrder.size);
    chunk.offset = offset;
    const end = offset + chunk.slots.length;
    if (end > maxOffsetEnd) maxOffsetEnd = end;
    const kids = (chunk.storyboard ? (children.get(chunk.storyboard.key) ?? []) : []).filter(
      (kid) => !placed.has(kid.chunk),
    );
    let r = rowStart;
    for (const kid of kids) r += place(kid.chunk, offset + chunk.slots.length + 1, r);
    const span = Math.max(1, r - rowStart);
    chunk.row = rowStart + (span - 1) / 2;
    return span;
  };
  let rootCursor = 0;
  for (const chunk of chunks) {
    if (placed.has(chunk)) continue;
    if (chunk.storyboard && isTarget.has(chunk.storyboard.key)) continue; // reached via its parent
    place(chunk, rootCursor, 0);
    rootCursor = maxOffsetEnd + 1;
  }
  for (const chunk of chunks) {
    // Targets whose parents never placed them (defensive): lay as roots.
    if (!placed.has(chunk)) {
      place(chunk, rootCursor, 0);
      rootCursor = maxOffsetEnd + 1;
    }
  }

  // ---- nodes & edges ---------------------------------------------------------
  // Instance ordinals along the chain: same type, numbered per moment.
  const totals = new Map<string, number>();
  for (const c of chunks)
    for (const slot of c.slots)
      for (const m of slot.members) totals.set(m.entity.key, (totals.get(m.entity.key) ?? 0) + 1);
  const seen = new Map<string, number>();
  const instanceFor = (e: Entity) => {
    const total = totals.get(e.key) ?? 0;
    if (total < 2) return undefined;
    const n = (seen.get(e.key) ?? 0) + 1;
    seen.set(e.key, n);
    return `#${n}/${total}`;
  };

  const nodes: Node[] = [];
  const edges: Edge[] = [];
  const nodeId = (chunk: Chunk, slot: number, member: number) => `seq-${chunks.indexOf(chunk)}-${slot}-${member}`;

  // Which cards of the previous moment continue into this one. A slot that
  // declares `observes` is entered only from the state it reads; a slot that
  // observes nothing on the board (its sources live elsewhere) falls back to
  // the full fan-out so no column is ever left orphaned.
  const entryPoints = (prev: Slot, slot: Slot): number[] => {
    const all = prev.members.map((_, i) => i);
    if (!slot.observes) return all;
    const observes = slot.observes;
    const matched = all.filter((i) => observes.includes(prev.members[i].entity.key));
    return matched.length > 0 ? matched : all;
  };

  const nextEdge = (id: string, source: string, target: string, data: BoardEdgeData): Edge => ({
    id,
    source,
    sourceHandle: 'r',
    target,
    targetHandle: 'l',
    animated: true,
    style: { stroke: '#a1a1aa', strokeWidth: 1.6 },
    zIndex: 3,
    data,
  });

  for (const chunk of chunks) {
    const baseY = chunk.row * rowH;
    if (chunk.storyboard && chunk.slots.length > 0) {
      nodes.push({
        id: `seq-sb:${chunk.storyboard.key}`,
        type: 'seqStoryboardHeader',
        position: { x: chunk.offset * STEP, y: baseY + SB_Y },
        style: { width: (chunk.slots.length - 1) * STEP + NODE_W, height: SB_H },
        data: { entity: chunk.storyboard },
        draggable: false,
        zIndex: 1,
      });
    }
    for (const seg of chunk.segments) {
      nodes.push({
        id: `seq-slice:${seg.slice.entity.key}`,
        type: 'seqSliceHeader',
        position: { x: (chunk.offset + seg.start) * STEP, y: baseY + SLICE_Y },
        style: { width: (seg.end - seg.start) * STEP + NODE_W, height: SLICE_H },
        data: {
          entity: seg.slice.entity,
          sliceKind: seg.slice.kind,
          status: trackingOf(model, seg.slice.entity)[0]?.status,
        },
        draggable: false,
        zIndex: 1,
      });
    }
    chunk.slots.forEach((slot, si) => {
      slot.members.forEach((m, mi) => {
        const cardY = baseY + ITEM_Y + mi * PITCH;
        nodes.push({
          id: nodeId(chunk, si, mi),
          type: 'seqSticky',
          position: { x: (chunk.offset + si) * STEP, y: cardY },
          style: { width: NODE_W, height: NODE_H },
          data: {
            entity: m.entity,
            selected: false,
            instance: instanceFor(m.entity),
            moment: m.moment?.entity,
            instanceUid: instanceUidOf(m.entity, m.moment?.entity),
          } satisfies SeqStickyData,
          draggable: false,
          zIndex: 2,
        });
        if (m.slice) {
          nodes.push({
            // Stacked projections share a column, so each one bands its own
            // card and the id carries the position it was drawn at.
            id: `seq-slice:${m.slice.entity.key}@${si}-${mi}`,
            type: 'seqSliceHeader',
            position: { x: (chunk.offset + si) * STEP, y: mi === 0 ? baseY + SLICE_Y : cardY - SLICE_H - 8 },
            style: { width: NODE_W, height: SLICE_H },
            data: {
              entity: m.slice.entity,
              sliceKind: m.slice.kind,
              status: trackingOf(model, m.slice.entity)[0]?.status,
            },
            draggable: false,
            zIndex: 1,
          });
        }
      });
      const sliceOfSlot = (mi: number) =>
        (slot.members[mi].slice ?? chunk.segments.find((seg) => si >= seg.start && si <= seg.end)?.slice)?.entity;

      // A screen is transparent to the chain: the moment after it continues
      // from the state it displayed, not from the screen.
      let pj = si - 1;
      while (pj >= 0 && chunk.slots[pj].transparent) pj--;
      if (pj >= 0) {
        // The previous moment fans out into every card of this one, unless
        // this moment named what it reads, which narrows the arrows to those
        // cards and leaves their same-moment siblings alone.
        const prev = chunk.slots[pj];
        for (const pi of entryPoints(prev, slot)) {
          for (let mi = 0; mi < slot.members.length; mi++) {
            edges.push(
              nextEdge(`${nodeId(chunk, si, mi)}-from-${pi}`, nodeId(chunk, pj, pi), nodeId(chunk, si, mi), {
                relation: 'happens next',
                doc: '',
                metadata: [],
                source: prev.members[pi].entity,
                target: slot.members[mi].entity,
                slice: sliceOfSlot(mi),
              }),
            );
          }
        }
      }
    });
  }

  // Fork edges ("we observe state, therefore we can act"): each road leaves
  // from the occurrence of the read model its entry observes. Fallbacks:
  // the transition's `after` event occurrence, then the chunk's last slot.
  // Cycles end forward in a loop stub, never a backward edge.
  for (const c of model.continuations) {
    const from = chunkBySb.get(c.from.key);
    const to = chunkBySb.get(c.to.key);
    if (!from || !to || from.slots.length === 0 || to.slots.length === 0) continue;
    let src: [number, number] = [from.slots.length - 1, 0];
    const entryRm = storyboardEntryReadModel(c.to);
    const entryRmKey = entryRm ? slugKey('readModel', entryRm) : undefined;
    const afterKey = slugKey('event', c.afterEvent);
    outer: for (let si = from.slots.length - 1; si >= 0; si--) {
      const members = from.slots[si].members;
      for (let mi = 0; mi < members.length; mi++) {
        const k = slugKey(members[mi].entity.kind, members[mi].entity.id);
        if (entryRmKey && k === entryRmKey) {
          src = [si, mi];
          break outer;
        }
        if (k === afterKey) src = [si, mi];
      }
    }
    const sourceEntity = from.slots[src[0]].members[src[1]].entity;
    // No guard text on the canvas: the amber dash already says "road";
    // the guard lives in the edge data (click → inspector, copy context).
    const forkStyle = {
      animated: true,
      style: { stroke: '#f59e0b', strokeWidth: 2, strokeDasharray: '7 5' },
      zIndex: 4,
    };
    const forkData = (target: Entity) =>
      ({
        relation: 'opens road',
        doc: c.doc,
        metadata: c.metadata,
        source: sourceEntity,
        target,
        slice: undefined,
      }) satisfies BoardEdgeData;
    const isBack = (placementOrder.get(to) ?? 0) <= (placementOrder.get(from) ?? 0);
    if (isBack) {
      const stubId = `seq-loop:${c.from.key}->${c.to.key}`;
      nodes.push({
        id: stubId,
        type: 'seqLoopStub',
        position: { x: (from.offset + from.slots.length) * STEP, y: from.row * rowH + ITEM_Y },
        style: { width: NODE_W, height: NODE_H },
        data: { entity: to.storyboard, doc: c.doc },
        draggable: false,
        zIndex: 2,
      });
      edges.push({
        id: `seq-fork:${c.from.key}->${c.to.key}`,
        source: nodeId(from, src[0], src[1]),
        sourceHandle: 'r',
        target: stubId,
        targetHandle: 'l',
        ...forkStyle,
        data: to.storyboard ? forkData(to.storyboard) : undefined,
      });
    } else {
      edges.push({
        id: `seq-fork:${c.from.key}->${c.to.key}`,
        source: nodeId(from, src[0], src[1]),
        sourceHandle: 'r',
        target: nodeId(to, 0, 0),
        targetHandle: 'l',
        ...forkStyle,
        data: forkData(to.slots[0].members[0].entity),
      });
    }
  }

  // No declared forks: keep the single linear chain across chunk boundaries.
  if (model.continuations.length === 0) {
    for (let k = 1; k < chunks.length; k++) {
      const prev = chunks[k - 1];
      const cur = chunks[k];
      if (prev.slots.length === 0 || cur.slots.length === 0) continue;
      edges.push(
        nextEdge(`seq-chunk-${k}`, nodeId(prev, prev.slots.length - 1, 0), nodeId(cur, 0, 0), {
          relation: 'happens next',
          doc: '',
          metadata: [],
          source: prev.slots[prev.slots.length - 1].members[0].entity,
          target: cur.slots[0].members[0].entity,
          slice: undefined,
        }),
      );
    }
  }

  return { nodes, edges };
}

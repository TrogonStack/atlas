// The code wall: the files that CodeRefAnnotations point at, laid out on a
// wall under the lane mesh. A code binding marks an entity as implemented,
// and from then on the code is the truth about it, so the wall shows where
// that truth lives. Files group by repo and their first two directories,
// because that is usually the module or service boundary. Each group sits
// as close under the cards that cite it as its neighbours allow, so a cite
// link runs mostly straight down.
//
// Only the annotation's own fields are read: there is no line number, and
// the file itself is never opened.

import type { Entity } from './model';
import { type Vec3, vec } from './ports';
import { type Region, regionOf } from './scene3d';

export interface CodeRef {
  repo: string;
  path: string;
  symbol?: string;
  ref?: string;
}

/** Complete code references only: without both a repo and a path a reference has no place on the wall. */
export function codeRefsOf(entity: Entity): CodeRef[] {
  return entity.annotations.flatMap((a) => {
    if (a.type !== 'CodeRefAnnotation') return [];
    const repo = typeof a.data.repo === 'string' ? a.data.repo.trim() : '';
    const path = typeof a.data.path === 'string' ? a.data.path.trim().replace(/^\/+/, '') : '';
    if (!repo || !path) return [];
    const symbol = typeof a.data.symbol === 'string' && a.data.symbol ? a.data.symbol : undefined;
    const ref = typeof a.data.ref === 'string' && a.data.ref ? a.data.ref : undefined;
    return [{ repo, path, symbol, ref }];
  });
}

/** The bundle a file belongs to: its repo and at most the first two directories of its path. */
export function bundleOf(ref: Pick<CodeRef, 'repo' | 'path'>): { id: string; repo: string; dir: string } {
  const dirs = ref.path.split('/').filter(Boolean).slice(0, -1).slice(0, 2);
  const dir = dirs.join('/');
  return { id: JSON.stringify([ref.repo, dir]), repo: ref.repo, dir };
}

export interface WallEntry {
  /** Entity key. */
  key: string;
  /** The x of the citing card. Its bundle is pulled under this point. */
  x: number;
  refs: CodeRef[];
}

export interface WallFile {
  id: string;
  repo: string;
  path: string;
  name: string;
  symbols: string[];
  bundle: string;
  citedBy: string[];
  center: Vec3;
  size: number;
}

export interface WallBundle {
  id: string;
  repo: string;
  dir: string;
  label: string;
  center: Vec3;
  width: number;
  height: number;
  files: string[];
}

export interface CodeWall {
  files: WallFile[];
  bundles: WallBundle[];
  region: Region;
}

export const FILE_SIZE = 10;
const CELL = 16;
const PAD = 7;
const LABEL_H = 9;
const BUNDLE_GAP = 14;

/**
 * Lays the cited files out in one row of bundles that hangs from `top` on
 * the plane `z`. Returns undefined when nothing cites a file, so a model
 * without code bindings has no wall at all.
 */
export function layoutCodeWall(entries: readonly WallEntry[], at: { top: number; z: number }): CodeWall | undefined {
  const { top: wallTop, z } = at;
  const files = new Map<string, { ref: CodeRef; symbols: Set<string>; citedBy: string[]; anchors: number[] }>();
  for (const entry of entries) {
    for (const ref of entry.refs) {
      const id = `file:${JSON.stringify([ref.repo, ref.path])}`;
      const f = files.get(id) ?? { ref, symbols: new Set<string>(), citedBy: [], anchors: [] };
      if (ref.symbol) f.symbols.add(ref.symbol);
      if (!f.citedBy.includes(entry.key)) {
        f.citedBy.push(entry.key);
        f.anchors.push(entry.x);
      }
      files.set(id, f);
    }
  }
  if (files.size === 0) return undefined;

  const groups = new Map<string, { info: ReturnType<typeof bundleOf>; files: string[]; anchors: number[] }>();
  for (const [id, f] of files) {
    const info = bundleOf(f.ref);
    const g = groups.get(info.id) ?? { info, files: [], anchors: [] };
    g.files.push(id);
    g.anchors.push(avg(f.anchors));
    groups.set(info.id, g);
  }
  const boxes = [...groups.values()]
    .map((g) => {
      const n = g.files.length;
      const cols = Math.min(n, Math.max(1, Math.ceil(Math.sqrt(n * 2))));
      const rows = Math.ceil(n / cols);
      return {
        g,
        cols,
        ideal: avg(g.anchors),
        x: 0,
        width: cols * CELL - (CELL - FILE_SIZE) + PAD * 2,
        height: rows * CELL - (CELL - FILE_SIZE) + PAD * 2 + LABEL_H,
      };
    })
    .sort((a, b) => a.ideal - b.ideal || a.g.info.id.localeCompare(b.g.info.id));
  packRow(boxes);

  const outFiles: WallFile[] = [];
  const bundles: WallBundle[] = [];
  for (const box of boxes) {
    const { info } = box.g;
    // Files in a bundle follow their cards left to right, so cite links do not cross inside it.
    const ordered = box.g.files
      .map((id, i) => ({ id, anchor: box.g.anchors[i] }))
      .sort((a, b) => a.anchor - b.anchor || a.id.localeCompare(b.id));
    const center = vec(box.x, wallTop - box.height / 2, z);
    const left = box.x - box.width / 2 + PAD + FILE_SIZE / 2;
    const top = wallTop - PAD - LABEL_H - FILE_SIZE / 2;
    ordered.forEach(({ id }, k) => {
      const f = files.get(id);
      if (!f) return;
      const col = k % box.cols;
      const row = Math.floor(k / box.cols);
      outFiles.push({
        id,
        repo: f.ref.repo,
        path: f.ref.path,
        name: f.ref.path.slice(f.ref.path.lastIndexOf('/') + 1),
        symbols: [...f.symbols].sort(),
        bundle: info.id,
        citedBy: f.citedBy,
        center: vec(left + col * CELL, top - row * CELL, z),
        size: FILE_SIZE,
      });
    });
    bundles.push({
      id: info.id,
      repo: info.repo,
      dir: info.dir,
      label: info.dir ? `${info.repo} · ${info.dir}` : info.repo,
      center,
      width: box.width,
      height: box.height,
      files: ordered.map((o) => o.id),
    });
  }
  const corners = bundles.flatMap((b) => [
    vec(b.center.x - b.width / 2, b.center.y - b.height / 2, z),
    vec(b.center.x + b.width / 2, b.center.y + b.height / 2, z),
  ]);
  return { files: outFiles, bundles, region: regionOf(corners, 12) };
}

/**
 * Places boxes in one row, in their given order, with no overlap and the
 * least total squared distance from each box's ideal x. Neighbours that
 * would overlap merge into one run, and a run sits at the mean of its
 * members' ideal places.
 */
function packRow(boxes: { ideal: number; x: number; width: number }[]): void {
  const runs: { items: number[]; offsets: number[]; x: number }[] = [];
  for (let i = 0; i < boxes.length; i++) {
    let run = { items: [i], offsets: [0], x: boxes[i].ideal };
    for (;;) {
      const prev = runs.at(-1);
      if (!prev) break;
      const last = prev.items[prev.items.length - 1];
      const prevRight = prev.x + prev.offsets[prev.offsets.length - 1] + boxes[last].width / 2;
      if (prevRight + BUNDLE_GAP <= run.x - boxes[run.items[0]].width / 2) break;
      runs.pop();
      let offset = prev.offsets[prev.offsets.length - 1];
      let before = last;
      for (const j of run.items) {
        offset += (boxes[before].width + boxes[j].width) / 2 + BUNDLE_GAP;
        prev.items.push(j);
        prev.offsets.push(offset);
        before = j;
      }
      prev.x = avg(prev.items.map((j, k) => boxes[j].ideal - prev.offsets[k]));
      run = prev;
    }
    runs.push(run);
  }
  for (const run of runs) run.items.forEach((j, k) => (boxes[j].x = run.x + run.offsets[k]));
}

function avg(xs: number[]): number {
  return xs.reduce((s, x) => s + x, 0) / Math.max(xs.length, 1);
}

// Text printed on a card face or a plate. WebGL has no text, so each face
// is drawn once into a canvas and used as a texture. The layout follows the
// Board's card: a small kind line, a title of up to three lines, and the
// slug, so a card reads the same in both views.

import { CanvasTexture, SRGBColorSpace } from 'three';
import { INK, MUTED_INK } from './palette';

// Canvas pixels per world unit. A Board card is 54 units wide, so its face
// is 270 pixels wide, a little sharper than the Board at 100% zoom.
const RES = 5;
const MAX_PX = 1024;

export interface Paint {
  /** World size of the face. */
  width: number;
  height: number;
  bg?: string;
  border?: string;
  dashed?: boolean;
  kicker?: string;
  title: string;
  sub?: string;
  /** World units, at the Board's sizes times 0.25. */
  titleSize?: number;
  lines?: number;
}

const MONO = 'ui-monospace, SFMono-Regular, Menlo, monospace';

function family(): string {
  return (typeof document !== 'undefined' && getComputedStyle(document.body).fontFamily) || 'system-ui, sans-serif';
}

/** Draws `p` into a new texture, or returns undefined when the browser gives no 2D canvas. */
export function paintTexture(p: Paint, anisotropy: number): CanvasTexture | undefined {
  const res = Math.min(RES, MAX_PX / p.width, MAX_PX / p.height);
  const w = Math.max(2, Math.round(p.width * res));
  const h = Math.max(2, Math.round(p.height * res));
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h;
  const g = canvas.getContext('2d');
  if (!g) return undefined;
  const u = res; // pixels per world unit
  const radius = 1.5 * u;
  const line = 0.5 * u;
  if (p.bg) {
    g.fillStyle = p.bg;
    roundRect(g, line / 2, line / 2, w - line, h - line, radius);
    g.fill();
  }
  if (p.border) {
    g.strokeStyle = p.border;
    g.lineWidth = line;
    g.setLineDash(p.dashed ? [2 * u, 1.2 * u] : []);
    roundRect(g, line / 2, line / 2, w - line, h - line, radius);
    g.stroke();
    g.setLineDash([]);
  }
  const padX = 3 * u;
  const maxW = w - padX * 2;
  let y = 2.2 * u;
  g.textBaseline = 'top';
  const fam = family();
  if (p.kicker) {
    g.fillStyle = MUTED_INK;
    g.font = `600 ${2.4 * u}px ${fam}`;
    g.fillText(clip(g, p.kicker.toUpperCase(), maxW), padX, y);
    y += 3.6 * u;
  }
  const size = (p.titleSize ?? 3.25) * u;
  g.fillStyle = INK;
  g.font = `600 ${size}px ${fam}`;
  const subH = p.sub ? 3.4 * u : 0;
  const room = Math.max(1, Math.floor((h - y - subH - 1.2 * u) / (size * 1.25)));
  const lines = wrap(g, p.title, maxW, Math.min(p.lines ?? 3, room));
  lines.forEach((text, i) => {
    g.fillText(text, padX, y + i * size * 1.25);
  });
  if (p.sub) {
    g.fillStyle = MUTED_INK;
    g.font = `400 ${2.4 * u}px ${MONO}`;
    g.fillText(clip(g, p.sub, maxW), padX, h - 2 * u - 2.4 * u);
  }
  const texture = new CanvasTexture(canvas);
  texture.colorSpace = SRGBColorSpace;
  texture.anisotropy = anisotropy;
  return texture;
}

function roundRect(g: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  g.beginPath();
  g.roundRect(x, y, w, h, Math.min(r, w / 2, h / 2));
}

/** Word wrap with a hard break for one long identifier, and an ellipsis on the last line that does not fit. */
function wrap(g: CanvasRenderingContext2D, text: string, maxW: number, maxLines: number): string[] {
  const out: string[] = [];
  let cur = '';
  const words = text.split(/\s+/).filter(Boolean);
  for (let i = 0; i < words.length; i++) {
    const word = words[i];
    const next = cur ? `${cur} ${word}` : word;
    if (g.measureText(next).width <= maxW) {
      cur = next;
      continue;
    }
    if (cur) out.push(cur);
    cur = word;
    // A single word wider than the card breaks where it must.
    while (g.measureText(cur).width > maxW && cur.length > 1) {
      let cut = cur.length - 1;
      while (cut > 1 && g.measureText(cur.slice(0, cut)).width > maxW) cut--;
      out.push(cur.slice(0, cut));
      cur = cur.slice(cut);
    }
    if (out.length >= maxLines) break;
  }
  if (cur && out.length < maxLines) out.push(cur);
  const clipped = out.slice(0, maxLines);
  const used = clipped.join(' ').replace(/\s+/g, '').length;
  if (used < text.replace(/\s+/g, '').length && clipped.length > 0) {
    clipped[clipped.length - 1] = clip(g, `${clipped[clipped.length - 1]}…`, maxW);
  }
  return clipped;
}

function clip(g: CanvasRenderingContext2D, text: string, maxW: number): string {
  if (g.measureText(text).width <= maxW) return text;
  let t = text;
  while (t.length > 1 && g.measureText(`${t}…`).width > maxW) t = t.slice(0, -1);
  return `${t}…`;
}

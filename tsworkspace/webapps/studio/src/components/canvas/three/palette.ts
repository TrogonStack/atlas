// Card colors for the 3D views. The Board colors a card with Tailwind
// classes from kindStyle; WebGL needs real colors, so this table gives the
// hex value of each class kindStyle uses. Reading kindStyle keeps one source
// for which kind gets which color: a kind that changes color on the Board
// changes here too, and a test fails if the new class has no hex value.

import { kindStyle } from '@/components/canvas/StickyNode';
import type { EntityKind } from '@/lib/model';

const TAILWIND_HEX: Record<string, string> = {
  white: '#ffffff',
  'amber-50': '#fffbeb',
  'amber-100': '#fef3c7',
  'amber-300': '#fcd34d',
  'amber-400': '#fbbf24',
  'cyan-50': '#ecfeff',
  'cyan-300': '#67e8f9',
  'emerald-50': '#ecfdf5',
  'emerald-200': '#a7f3d0',
  'emerald-300': '#6ee7b7',
  'emerald-400': '#34d399',
  'fuchsia-50': '#fdf4ff',
  'fuchsia-100': '#fae8ff',
  'fuchsia-300': '#f0abfc',
  'indigo-50': '#eef2ff',
  'indigo-200': '#c7d2fe',
  'indigo-300': '#a5b4fc',
  'lime-50': '#f7fee7',
  'lime-300': '#bef264',
  'orange-200': '#fed7aa',
  'orange-400': '#fb923c',
  'red-50': '#fef2f2',
  'red-100': '#fee2e2',
  'red-300': '#fca5a5',
  'red-400': '#f87171',
  'rose-50': '#fff1f2',
  'rose-300': '#fda4af',
  'sky-50': '#f0f9ff',
  'sky-200': '#bae6fd',
  'sky-300': '#7dd3fc',
  'sky-400': '#38bdf8',
  'slate-50': '#f8fafc',
  'slate-100': '#f1f5f9',
  'slate-300': '#cbd5e1',
  'slate-400': '#94a3b8',
  'stone-100': '#f5f5f4',
  'stone-400': '#a8a29e',
  'teal-50': '#f0fdfa',
  'teal-300': '#5eead4',
  'violet-50': '#f5f3ff',
  'violet-200': '#ddd6fe',
  'violet-300': '#c4b5fd',
  'violet-400': '#a78bfa',
  'yellow-50': '#fefce8',
  'yellow-100': '#fef9c3',
  'yellow-300': '#fde047',
  'zinc-50': '#fafafa',
  'zinc-100': '#f4f4f5',
  'zinc-200': '#e4e4e7',
  'zinc-300': '#d4d4d8',
  'zinc-400': '#a1a1aa',
};

/** The hex value of a `bg-*` or `border-*` class, or undefined when the table lacks it. */
export function hexOfClass(cls: string): string | undefined {
  return TAILWIND_HEX[cls.replace(/^(bg|border)-/, '')];
}

export interface KindColors {
  bg: string;
  border: string;
  label: string;
}

export function kindColors(kind: EntityKind): KindColors {
  const style = kindStyle(kind);
  return {
    bg: hexOfClass(style.bg) ?? TAILWIND_HEX['zinc-100'],
    border: hexOfClass(style.border) ?? TAILWIND_HEX['zinc-300'],
    label: style.label,
  };
}

/** The colors the Board puts on a card the validator flags, in place of the kind's own border. */
export const FLAG_BORDER: Record<'pending' | 'error' | 'orphan', string> = {
  pending: '#f59e0b',
  error: '#ef4444',
  orphan: '#f59e0b',
};

export const INK = '#18181b';
export const MUTED_INK = '#71717a';
export const FOCUS = '#2563eb';

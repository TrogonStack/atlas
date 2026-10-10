// Centralized branch-context helpers (Phase 3: Studio).
//
// The active branch lives entirely in the URL as `?branch=<name>`: there is
// no server session, so every read must carry it explicitly and every link
// must republish it or a click-through silently reverts to baseline.
//
// The codebase has two parallel URL-state idioms (see ModelShell.tsx/
// SequenceBoard.tsx which use nuqs, vs. OverviewPage.tsx/ScreensBoard.tsx which
// hand-rolls URLSearchParams + history.replaceState). This module supports
// both: `useBranchParam` for nuqs-tree components, and `readBranchFromUrl` /
// `withBranchQuery` for plain string/href construction anywhere else
// (including outside React, e.g. building an href in a render function).
import { parseAsString, useQueryState } from 'nuqs';
import type { BranchDiffEntry } from '@/lib/api';
import { type EntityKind, slugKey, wireKind } from '@/lib/model';
import { isSafeBranchName } from '../../shared/safe-namespace.mjs';

/** nuqs param definition for `?branch=`, shared so every reader/writer agrees on the key. */
const branchParser = parseAsString;

/**
 * Current-branch accessor for components inside the `NuqsAdapter` tree
 * (ModelShell, Board, SequenceBoard, TopBar, ...). Returns `undefined` when
 * absent (baseline) or when the URL value fails `isSafeBranchName`, a
 * malformed/hostile `?branch=` is treated as "no branch" rather than
 * forwarded to the API.
 */
export function useBranchParam(): [string | undefined, (next: string | undefined) => void] {
  const [raw, setRaw] = useQueryState('branch', branchParser);
  const value = raw && isSafeBranchName(raw) ? raw : undefined;
  const set = (next: string | undefined) => setRaw(next && next.length > 0 ? next : null);
  return [value, set];
}

/**
 * Reads `?branch=` from `window.location.search` for call sites that don't
 * sit inside the nuqs tree (OverviewPage's manual URLSearchParams idiom).
 * Same validation as `useBranchParam`.
 */
export function readBranchFromUrl(): string | undefined {
  if (typeof window === 'undefined') return undefined;
  const raw = new URLSearchParams(window.location.search).get('branch');
  return raw && isSafeBranchName(raw) ? raw : undefined;
}

/**
 * Appends (or removes) `?branch=` on an arbitrary href, preserving any
 * existing query string. Used at every hard-navigation site (`<a href>`,
 * `window.location.href = ...`) so a branch preview survives a full page
 * load, which nuqs alone cannot do.
 */
export function withBranchQuery(href: string, branch: string | undefined): string {
  const [path, hash] = href.split('#');
  const [base, query] = path.split('?');
  const params = new URLSearchParams(query ?? '');
  if (branch) params.set('branch', branch);
  else params.delete('branch');
  const qs = params.toString();
  return `${base}${qs ? `?${qs}` : ''}${hash ? `#${hash}` : ''}`;
}

export type BranchStatus = 'added' | 'changed' | 'deleted' | 'conflict';

// Mirrors `BranchDiffEntry.Status` in service.proto. The three
// STATUS_CONFLICT_* variants (edit/edit, edit/delete, delete/edit) all
// render as the same "conflict" badge: the three-pane conflict view is
// where the distinction actually matters. STATUS_CONVERGED has nothing to
// show (baseline caught up to `ours`; the proto doc says it's dropped
// silently on merge) and STATUS_UNSPECIFIED/unknown values are ignored.
export function branchEntryStatus(wireStatus: string): BranchStatus | undefined {
  switch (wireStatus) {
    case 'STATUS_ADDED':
      return 'added';
    case 'STATUS_CHANGED':
      return 'changed';
    case 'STATUS_DELETED':
      return 'deleted';
    case 'STATUS_CONFLICT_EDIT_EDIT':
    case 'STATUS_CONFLICT_EDIT_DELETE':
    case 'STATUS_CONFLICT_DELETE_EDIT':
      return 'conflict';
    default:
      return undefined;
  }
}

/**
 * Builds a `(kind, namespace, slug)` → status lookup from a `/api/branch-diff`
 * response, ignoring version (mirrors `slugKey`'s precedent in multiboard.ts
 * for post-hoc node-to-entity matching: a diff entry targets "the entity",
 * not one specific version pinned in a rendered node).
 */
export function branchStatusLookup(entries: BranchDiffEntry[]): Map<string, BranchStatus> {
  const out = new Map<string, BranchStatus>();
  for (const entry of entries) {
    const kind = wireKind(entry.ref?.kind);
    const id = entry.ref?.id;
    if (!kind || !id?.namespace || !id?.slug) continue;
    const status = branchEntryStatus(entry.status);
    if (!status) continue;
    out.set(slugKey(kind, { namespace: id.namespace, slug: id.slug }), status);
  }
  return out;
}

/** Looks up the branch status for one entity, by kind/namespace/slug. */
export function lookupBranchStatus(
  lookup: Map<string, BranchStatus>,
  kind: EntityKind,
  id: { namespace: string; slug: string },
): BranchStatus | undefined {
  return lookup.get(slugKey(kind, id));
}

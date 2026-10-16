// Centralized branch-context helpers (Phase 3: Studio).
//
// The active branch lives entirely in the URL as `?branch=<name>`: there is
// no server session, so every read must carry it explicitly and every link
// must republish it or a click-through silently reverts to baseline.
//
// `?asOf=<changeset id>` lives next to it: a read-only view of baseline right
// after that changeset. The server rewinds baseline history only, so the two
// are mutually exclusive: setting one clears the other, and a URL that
// carries both reads as `asOf`.
//
// The codebase has two parallel URL-state idioms (see ModelShell.tsx/
// SequenceBoard.tsx which use nuqs, vs. OverviewPage.tsx/ScreensBoard.tsx which
// hand-rolls URLSearchParams + history.replaceState). This module supports
// both: `useBranchParam` / `useAsOfParam` for nuqs-tree components, and
// `readBranchFromUrl` / `readAsOfFromUrl` / `withBranchQuery` /
// `withAsOfQuery` for plain string/href construction anywhere else
// (including outside React, e.g. building an href in a render function).
import { parseAsString, useQueryStates } from 'nuqs';
import type { BranchDiffEntry } from '@/lib/api';
import { type EntityKind, slugKey, wireKind } from '@/lib/model';
import { isChangesetId, isSafeBranchName } from '../../shared/safe-namespace.mjs';

/** nuqs param definitions for `?branch=` and `?asOf=`, shared so every reader/writer agrees on the keys. */
const boardRefParsers = { branch: parseAsString, asOf: parseAsString };

function validAsOf(raw: string | null | undefined): string | undefined {
  return raw && isChangesetId(raw) ? raw : undefined;
}

function validBranch(raw: string | null | undefined, asOf: string | null | undefined): string | undefined {
  return raw && isSafeBranchName(raw) && !validAsOf(asOf) ? raw : undefined;
}

/**
 * Current-branch accessor for components inside the `NuqsAdapter` tree
 * (ModelShell, Board, SequenceBoard, TopBar, ...). Returns `undefined` when
 * absent (baseline) or when the URL value fails `isSafeBranchName`, a
 * malformed/hostile `?branch=` is treated as "no branch" rather than
 * forwarded to the API. Setting a branch clears `?asOf=`.
 */
export function useBranchParam(): [string | undefined, (next: string | undefined) => void] {
  const [raw, setRaw] = useQueryStates(boardRefParsers);
  const value = validBranch(raw.branch, raw.asOf);
  const set = (next: string | undefined) => setRaw({ branch: next && next.length > 0 ? next : null, asOf: null });
  return [value, set];
}

/**
 * Current `?asOf=` changeset id inside the `NuqsAdapter` tree. Anything that
 * is not a lowercase hyphenated UUID reads as unset. Setting it clears `?branch=`.
 */
export function useAsOfParam(): [string | undefined, (next: string | undefined) => void] {
  const [raw, setRaw] = useQueryStates(boardRefParsers);
  const value = validAsOf(raw.asOf);
  const set = (next: string | undefined) => setRaw(next ? { asOf: next, branch: null } : { asOf: null });
  return [value, set];
}

/**
 * Reads `?branch=` from `window.location.search` for call sites that don't
 * sit inside the nuqs tree (OverviewPage's manual URLSearchParams idiom).
 * Same validation as `useBranchParam`.
 */
export function readBranchFromUrl(): string | undefined {
  if (typeof window === 'undefined') return undefined;
  const params = new URLSearchParams(window.location.search);
  return validBranch(params.get('branch'), params.get('asOf'));
}

/** Reads `?asOf=` from `window.location.search`. Same validation as `useAsOfParam`. */
export function readAsOfFromUrl(): string | undefined {
  if (typeof window === 'undefined') return undefined;
  return validAsOf(new URLSearchParams(window.location.search).get('asOf'));
}

function withParam(href: string, edit: (params: URLSearchParams) => void): string {
  const [path, hash] = href.split('#');
  const [base, query] = path.split('?');
  const params = new URLSearchParams(query ?? '');
  edit(params);
  const qs = params.toString();
  return `${base}${qs ? `?${qs}` : ''}${hash ? `#${hash}` : ''}`;
}

/**
 * Appends (or removes) `?branch=` on an arbitrary href, preserving any
 * existing query string. Used at every hard-navigation site (`<a href>`,
 * `window.location.href = ...`) so a branch preview survives a full page
 * load, which nuqs alone cannot do. A branch drops `?asOf=`.
 */
export function withBranchQuery(href: string, branch: string | undefined): string {
  return withParam(href, (params) => {
    if (branch) {
      params.set('branch', branch);
      params.delete('asOf');
    } else params.delete('branch');
  });
}

/** Like `withBranchQuery`, for `?asOf=`. An as-of view drops `?branch=`. */
export function withAsOfQuery(href: string, asOf: string | undefined): string {
  return withParam(href, (params) => {
    if (asOf) {
      params.set('asOf', asOf);
      params.delete('branch');
    } else params.delete('asOf');
  });
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

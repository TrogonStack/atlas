// Maps cross-context entity refs to their "home" EventModels: the EMs
// that claim each entity as a member. Used by the Inspector to render
// "Open in <em>" jump links on boundary read models so the reader can
// follow a subscription out to the upstream context's curation.
//
// Caches the indexed lookup for the page lifetime. We only cache successful
// loads: a rejected promise would otherwise stick for the whole session
// and break every cross-context jump link until the user reloaded.
// `invalidateEntityHomes()` lets realtime callers drop the cache when an
// EventModel mutation lands so the index reflects new memberships.
import { useEffect, useMemo, useState } from 'react';
import { api } from '@/lib/api';
import { readBranchFromUrl } from '@/lib/branch';
import { buildModel, type Entity, type EntityId, wireKind } from '@/lib/model';

interface Index {
  // key: `${entityKind}:${namespace}/${slug}` → EventModel entities
  homes: Map<string, Entity[]>;
}

// Cache is keyed by branch (empty string = baseline) so a branch switch
// cannot reuse memberships discovered on a different overlay.
const cacheByBranch = new Map<string, Promise<Index>>();

async function loadIndex(branch: string | undefined): Promise<Index> {
  const cacheKey = branch ?? '';
  const existing = cacheByBranch.get(cacheKey);
  if (existing) return existing;
  const promise = (async () => {
    const res = await api.eventModels(branch ? { branch } : undefined);
    const model = buildModel(res.entities ?? []);
    const homes = new Map<string, Entity[]>();
    for (const em of model.entities) {
      if (em.kind !== 'eventModel') continue;
      const members = Array.isArray(em.raw.members) ? em.raw.members : [];
      for (const m of members) {
        const rec = m as Record<string, unknown>;
        const id = rec.id as Record<string, unknown> | undefined;
        const kind = wireKind(rec.kind);
        if (!kind || !id?.namespace || !id?.slug) continue;
        const key = `${kind}:${id.namespace}/${id.slug}`;
        const list = homes.get(key) ?? [];
        list.push(em);
        homes.set(key, list);
      }
    }
    return { homes };
  })();
  cacheByBranch.set(cacheKey, promise);
  // Drop the cache on failure so the next caller retries instead of being
  // stuck with a permanently-rejected promise.
  promise.catch(() => {
    if (cacheByBranch.get(cacheKey) === promise) cacheByBranch.delete(cacheKey);
  });
  return promise;
}

export function invalidateEntityHomes(): void {
  cacheByBranch.clear();
}

/**
 * For each ref (kind + EntityId), returns the EventModels claiming it
 * as a member. Empty array while loading or when nothing matches.
 */
export function useEntityHomes(refs: { kind: string; id: EntityId }[]): {
  homes: Map<string, Entity[]>;
  error: string | undefined;
  retry: () => void;
} {
  const [index, setIndex] = useState<Index>();
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  const branch = readBranchFromUrl();
  // biome-ignore lint/correctness/useExhaustiveDependencies: attempt is an intentional retry trigger
  useEffect(() => {
    let alive = true;
    setError(undefined);
    loadIndex(branch)
      .then((idx) => {
        if (alive) setIndex(idx);
      })
      .catch((e) => {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      alive = false;
    };
  }, [attempt, branch]);
  const homes = useMemo(() => {
    const out = new Map<string, Entity[]>();
    if (!index) return out;
    for (const ref of refs) {
      const key = `${ref.kind}:${ref.id.namespace}/${ref.id.slug}`;
      const list = index.homes.get(key);
      if (list && list.length > 0) {
        out.set(`${ref.id.namespace}/${ref.id.slug}`, list);
      }
    }
    return out;
  }, [index, refs]);
  const retry = () => {
    invalidateEntityHomes();
    setAttempt((n) => n + 1);
  };
  return { homes, error, retry };
}

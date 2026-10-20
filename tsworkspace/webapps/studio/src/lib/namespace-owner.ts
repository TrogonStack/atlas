import type { NamespaceRegistryRow } from './api';

/** The owner a namespace gets when none is named. Matches the server. */
export const DEFAULT_OWNER = 'default';

export function ownerLabel(row: NamespaceRegistryRow): string {
  return row.parent === '' ? DEFAULT_OWNER : row.parent;
}

export interface OwnerGroup<T> {
  /**
   * Null for namespaces with no recorded owner, or for everything when the
   * registry could not be read. Null keeps a real owner named "unregistered" apart.
   */
  owner: string | null;
  namespaces: { namespace: string; items: T[] }[];
}

/**
 * Groups items by registry owner, then by namespace, both sorted, with the
 * ownerless group last. Item order inside a namespace is kept. Without a
 * registry everything lands in that one ownerless group, so the caller can
 * fall back to the plain by-namespace list.
 */
export function groupByOwner<T>(
  items: readonly T[],
  namespaceOf: (item: T) => string,
  registry: readonly NamespaceRegistryRow[] | undefined,
): OwnerGroup<T>[] {
  // Entity keys carry the namespace id; unregistered rows have no id, only a name.
  const ownerOf = new Map<string, string | null>();
  for (const r of registry ?? []) {
    const owner = r.registered ? ownerLabel(r) : null;
    if (r.id !== '') ownerOf.set(r.id, owner);
    if (!ownerOf.has(r.name)) ownerOf.set(r.name, owner);
  }

  const tree = new Map<string | null, Map<string, T[]>>();
  for (const item of items) {
    const ns = namespaceOf(item);
    const owner = ownerOf.get(ns) ?? null;
    const byNs = tree.get(owner) ?? new Map<string, T[]>();
    const list = byNs.get(ns) ?? [];
    list.push(item);
    byNs.set(ns, list);
    tree.set(owner, byNs);
  }

  return [...tree.keys()]
    .sort((a, b) => (a === null ? 1 : b === null ? -1 : a.localeCompare(b)))
    .map((owner) => {
      const byNs = tree.get(owner) ?? new Map<string, T[]>();
      return {
        owner,
        namespaces: [...byNs.keys()].sort().map((namespace) => ({ namespace, items: byNs.get(namespace) ?? [] })),
      };
    });
}

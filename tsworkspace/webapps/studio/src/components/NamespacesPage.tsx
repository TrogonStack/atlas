// The namespace registry, as a screen.
//
// Everything else in the studio treats a namespace as a string to filter on.
// This is the one place that treats it as a row with an owner, because the
// registry is what decides which namespaces a key can see at all. It is an
// observation surface: agents register and move namespaces through the CLI
// or MCP, and this screen shows the outcome.
//
// Three things are worth seeing here and nowhere else:
//
//   - the id, which is what entity keys are actually built from. A namespace
//     adopted before the registry has an id equal to its name; one minted by
//     RegisterNamespace has an opaque `ns_...`. An agent moving a namespace
//     needs the id, and there is no other surface that reports it.
//   - the owner, which is the whole of the authorization answer when SpiceDB
//     is off.
//   - whether the namespace is registered at all. The directory fails closed
//     on an unregistered namespace: it is visible to an unrestricted caller
//     and to nobody else, so a client with a scoped key sees it as simply not
//     existing. That is invisible from every other screen.
import { useCallback, useEffect, useMemo, useState } from 'react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { api, type NamespaceRegistryRow } from '@/lib/api';

/** The owner a namespace gets when none is named. Matches the server. */
const DEFAULT_OWNER = 'default';

function ownerLabel(row: NamespaceRegistryRow): string {
  return row.parent === '' ? DEFAULT_OWNER : row.parent;
}

export function NamespacesPage() {
  const [rows, setRows] = useState<NamespaceRegistryRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState('');

  const load = useCallback(async (signal?: AbortSignal) => {
    try {
      const res = await api.namespaceRegistry({ signal });
      if (signal?.aborted) return;
      setRows(res.namespaces ?? []);
      setError(null);
    } catch (e) {
      if (signal?.aborted || (e as Error)?.name === 'AbortError') return;
      setError((e as Error).message);
    }
  }, []);

  useEffect(() => {
    const ctrl = new AbortController();
    void load(ctrl.signal);
    return () => ctrl.abort();
  }, [load]);

  const visible = useMemo(() => {
    if (rows == null) return [];
    const q = filter.trim().toLowerCase();
    if (q === '') return rows;
    return rows.filter(
      (r) =>
        r.name.toLowerCase().includes(q) || ownerLabel(r).toLowerCase().includes(q) || r.id.toLowerCase().includes(q),
    );
  }, [rows, filter]);

  const unregistered = useMemo(() => (rows ?? []).filter((r) => !r.registered), [rows]);

  return (
    <div className="flex min-h-screen flex-col gap-6 bg-background p-6">
      <header className="flex flex-col gap-1">
        <div className="flex items-center gap-3">
          <a href="/" className="text-xs text-muted-foreground hover:text-foreground">
            &lsaquo; Overview
          </a>
          <h1 className="text-sm font-semibold tracking-tight">Namespaces</h1>
          <Button variant="outline" size="sm" className="ml-auto" onClick={() => void load()}>
            Refresh
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          A namespace belongs to exactly one owner, and a key sees only what its owner owns.
        </p>
      </header>

      {error ? (
        <output
          aria-live="polite"
          className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700"
        >
          {error}
        </output>
      ) : null}

      {unregistered.length > 0 ? (
        <section className="flex flex-col gap-2 rounded-md border border-amber-200 bg-amber-50 px-3 py-3">
          <h2 className="text-xs font-semibold text-amber-900">
            {unregistered.length} namespace{unregistered.length === 1 ? '' : 's'} hold entities but have no registry row
          </h2>
          <p className="text-xs text-amber-800">
            Nobody owns these, so a key scoped to an owner cannot see them at all. An agent claiming one through the CLI
            or MCP records an owner and makes it visible to that owner.
          </p>
          <ul className="flex flex-wrap gap-2 text-xs text-amber-900">
            {unregistered.map((r) => (
              <li key={r.name} className="font-mono">
                {r.name}
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <section className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <h2 className="text-xs font-semibold">Registry</h2>
          <Input
            className="ml-auto w-64"
            placeholder="Filter by name, owner, or id"
            aria-label="filter namespaces"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>

        {rows == null ? (
          <output aria-live="polite" className="text-xs text-muted-foreground">
            Loading…
          </output>
        ) : visible.length === 0 ? (
          <p className="text-xs text-muted-foreground">
            {rows.length === 0 ? 'No namespaces yet.' : 'No namespace matches that filter.'}
          </p>
        ) : (
          <table className="w-full border-collapse text-xs">
            <thead>
              <tr className="border-b border-border text-left text-muted-foreground">
                <th className="py-2 font-medium">Name</th>
                <th className="py-2 font-medium">Owner</th>
                <th className="py-2 font-medium">Id</th>
                <th className="py-2 text-right font-medium">Entities</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((row) => (
                <tr key={row.id || `unregistered:${row.name}`} className="border-b border-border/60">
                  <td className="py-2">
                    <span className="font-medium">{row.name}</span>
                    {row.registered ? null : (
                      <Badge variant="outline" className="ml-2 border-amber-300 text-amber-700">
                        unregistered
                      </Badge>
                    )}
                  </td>
                  <td className="py-2 text-muted-foreground">{row.registered ? ownerLabel(row) : 'no owner'}</td>
                  <td className="py-2 font-mono text-[11px] text-muted-foreground">{row.id || 'not minted'}</td>
                  <td className="py-2 text-right tabular-nums text-muted-foreground">{row.entityCount}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <p className="text-xs text-muted-foreground">
          Registering, claiming, and moving namespaces are agent operations. A move re-parents one registry row and
          rewrites no entities, and the server admits it from an unrestricted agent key only.
        </p>
      </section>
    </div>
  );
}

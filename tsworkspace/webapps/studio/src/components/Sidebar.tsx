import { Search } from 'lucide-react';
import { useEffect, useState } from 'react';
import { kindStyle } from '@/components/canvas/StickyNode';
import { Badge } from '@/components/ui/badge';
import { Input } from '@/components/ui/input';
import { Separator } from '@/components/ui/separator';
import { api } from '@/lib/api';
import { buildModel, type Entity, type Model } from '@/lib/model';

const LEGEND: { kind: Entity['kind']; label: string; swatch: string }[] = [
  { kind: 'event', label: 'Event', swatch: 'bg-amber-300' },
  { kind: 'command', label: 'Command', swatch: 'bg-sky-300' },
  { kind: 'readModel', label: 'Read Model', swatch: 'bg-emerald-300' },
  { kind: 'processor', label: 'Automation', swatch: 'bg-violet-300' },
  { kind: 'ui', label: 'UI', swatch: 'bg-zinc-200' },
  { kind: 'persona', label: 'Persona', swatch: 'bg-yellow-200' },
  { kind: 'component', label: 'Component', swatch: 'bg-zinc-100' },
  { kind: 'externalSystem', label: 'External System', swatch: 'bg-fuchsia-200' },
  { kind: 'tracker', label: 'Tracker', swatch: 'bg-stone-200' },
  { kind: 'boundedContext', label: 'Bounded Context', swatch: 'bg-fuchsia-100' },
  { kind: 'domain', label: 'Domain', swatch: 'bg-indigo-100' },
  { kind: 'subdomain', label: 'Subdomain', swatch: 'bg-indigo-50' },
  { kind: 'schema', label: 'Schema', swatch: 'bg-slate-100' },
  { kind: 'project', label: 'Project', swatch: 'bg-cyan-100' },
  { kind: 'screen', label: 'Screen', swatch: 'bg-teal-100' },
  { kind: 'term', label: 'Term', swatch: 'bg-lime-100' },
  { kind: 'ambiguity', label: 'Ambiguity', swatch: 'bg-rose-100' },
];

export function Sidebar({
  model,
  namespaces,
  allNamespaces,
  namespaceLabels,
  branch,
  onNamespaces,
  onSelect,
}: {
  model: Model;
  namespaces: string[];
  allNamespaces: string[];
  // A namespace travels as its id everywhere (the URL, every filter), but
  // an id minted by the registry is opaque. Missing entries fall back to
  // the id, which is also the right answer for a namespace that predates
  // the registry: there, the id is the name.
  namespaceLabels: Record<string, string>;
  // Search runs against the branch's own index, so a preview that shows an
  // entity on the board must also find it here.
  branch: string | undefined;
  onNamespaces: (ns: string[]) => void;
  onSelect: (key: string) => void;
}) {
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<Entity[]>([]);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string>();
  const [retryCount, setRetryCount] = useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: retryCount is an intentional retry trigger
  useEffect(() => {
    if (!query.trim()) {
      setHits([]);
      setSearchError(undefined);
      return;
    }
    let ctrl: AbortController | undefined;
    const t = setTimeout(async () => {
      ctrl = new AbortController();
      setSearching(true);
      setSearchError(undefined);
      try {
        const res = await api.search(query, namespaces, { signal: ctrl.signal, branch });
        const entities = buildModel(
          (res.results ?? []).map((h) => h.entity).filter((e): e is NonNullable<typeof e> => Boolean(e)),
        ).entities;
        setHits(entities);
      } catch (e) {
        if (e instanceof Error && e.name === 'AbortError') return;
        setHits([]);
        setSearchError(e instanceof Error ? e.message : String(e));
      } finally {
        setSearching(false);
      }
    }, 250);
    return () => {
      clearTimeout(t);
      ctrl?.abort();
    };
  }, [query, namespaces, retryCount, branch]);

  const counts = new Map<string, number>();
  for (const e of model.entities) counts.set(e.kind, (counts.get(e.kind) ?? 0) + 1);

  // The full namespace list survives filtering (fetched unfiltered), so a
  // deselected context can always be brought back.
  const options = allNamespaces.length > 0 ? allNamespaces : model.namespaces;
  const toggle = (ns: string) =>
    onNamespaces(namespaces.includes(ns) ? namespaces.filter((n) => n !== ns) : [...namespaces, ns]);

  return (
    <aside className="flex h-full w-64 flex-col border-r border-border bg-card">
      <div className="space-y-3 p-4">
        <div>
          <div className="mb-1 flex items-baseline justify-between">
            <span className="text-xs font-semibold uppercase text-muted-foreground">Namespaces</span>
            {namespaces.length > 0 ? (
              <button
                type="button"
                onClick={() => onNamespaces([])}
                className="text-[10px] font-medium text-muted-foreground underline-offset-2 hover:underline"
              >
                all
              </button>
            ) : null}
          </div>
          <div className="space-y-1 rounded-md border border-input p-2">
            {options.map((ns) => (
              <label key={ns} className="flex cursor-pointer items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  className="h-3.5 w-3.5 accent-foreground"
                  checked={namespaces.length === 0 || namespaces.includes(ns)}
                  onChange={() => {
                    // From "all" (nothing checked = everything), unchecking
                    // one means "everything except it".
                    if (namespaces.length === 0) onNamespaces(options.filter((n) => n !== ns));
                    else toggle(ns);
                  }}
                />
                <span className="truncate">{namespaceLabels[ns] ?? ns}</span>
              </label>
            ))}
            {options.length === 0 ? <div className="text-xs text-muted-foreground">no namespaces</div> : null}
          </div>
          {namespaces.length === 0 ? (
            <div className="mt-1 text-[10px] text-muted-foreground">all namespaces shown</div>
          ) : null}
        </div>
        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            aria-label="Search the model"
            placeholder="Search the model…"
            className="pl-8"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
      </div>
      <Separator />
      <div className="flex-1 overflow-y-auto p-4">
        {query.trim() ? (
          <section>
            <h3 className="mb-2 text-xs font-semibold uppercase text-muted-foreground">
              {searching ? 'Searching…' : `Results (${hits.length})`}
            </h3>
            {searchError ? (
              <div
                role="alert"
                className="mb-2 rounded-md border border-red-300 bg-red-50 px-2 py-1 text-xs text-red-700"
              >
                {searchError}
                <button type="button" onClick={() => setRetryCount((n) => n + 1)} className="ml-2 underline">
                  Retry
                </button>
              </div>
            ) : null}
            <ul className="space-y-1">
              {hits.map((e) => (
                <li key={e.key}>
                  <button
                    type="button"
                    className="w-full rounded-md border border-border px-2 py-1.5 text-left text-xs hover:bg-accent"
                    onClick={() => onSelect(e.key)}
                  >
                    <Badge variant="secondary" className="mr-1.5">
                      {kindStyle(e.kind).label}
                    </Badge>
                    {e.title}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ) : (
          <section>
            <h3 className="mb-2 text-xs font-semibold uppercase text-muted-foreground">Legend</h3>
            <ul className="space-y-1.5">
              {LEGEND.map((l) => (
                <li key={l.kind} className="flex items-center gap-2 text-sm">
                  <span className={`h-3 w-3 rounded-sm border border-black/10 ${l.swatch}`} />
                  {l.label}
                  <span className="ml-auto font-mono text-xs text-muted-foreground">{counts.get(l.kind) ?? 0}</span>
                </li>
              ))}
            </ul>
            <h3 className="mb-2 mt-6 text-xs font-semibold uppercase text-muted-foreground">Slices</h3>
            <div className="text-sm text-muted-foreground">
              {model.slices.length} slices · {model.storyboards.length} storyboards
            </div>
          </section>
        )}
      </div>
    </aside>
  );
}

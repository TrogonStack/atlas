// Overview, the studio's landing surface (Decision: the studio is navigated,
// not filtered). Three sections:
//   1. Core Domain Chart of the problem-space knowledge graph (lightweight).
//   2. EventModel index, grouped by namespace, each card linking to /em/...
//   3. (later) Context map.
// Renders without loading the whole store: only EventModel entities and
// the problem-space namespaces (Decision #29).
import { ReactFlowProvider } from '@xyflow/react';
import { Search } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import type { Selection } from '@/components/canvas/Board';
import { ContextMapBoard } from '@/components/canvas/ContextMapBoard';
import { DomainChartBoard } from '@/components/canvas/DomainChartBoard';
import { ErrorBoundary } from '@/components/ErrorBoundary';
import { api } from '@/lib/api';
import { readBranchFromUrl, withBranchQuery } from '@/lib/branch';
import {
  asId,
  buildModel,
  classificationOf,
  domainOf,
  type Entity,
  type Model,
  realizesOf,
  wireKind,
} from '@/lib/model';
import { cn } from '@/lib/utils';

const STORAGE_KEY = 'overview:filter:domains';
const MODE_STORAGE_KEY = 'overview:mode';

// Overview-level visualizations of the same data. Each is the whole overview
// re-shaped, not a filter. Distinct from the per-model `?view=` on the
// ModelShell tabs, which are scoped to one event model.
const OVERVIEW_MODES = ['chart', 'contexts', 'models'] as const;
type OverviewMode = (typeof OVERVIEW_MODES)[number];

const OVERVIEW_MODE_LABELS: Record<OverviewMode, string> = {
  chart: 'Domain chart',
  contexts: 'Contexts',
  models: 'Models',
};

const KIND_SHORT: Record<Entity['kind'], string> = {
  event: 'event',
  command: 'command',
  readModel: 'read_model',
  processor: 'processor',
  ui: 'ui',
  persona: 'persona',
  swimlane: 'swimlane',
  commandSlice: 'command_slice',
  readModelSlice: 'read_model_slice',
  automationSlice: 'automation_slice',
  uiSlice: 'ui_slice',
  storyboard: 'storyboard',
  eventModel: 'event_model',
  component: 'component',
  externalSystem: 'external_system',
  tracker: 'tracker',
  boundedContext: 'bounded_context',
  domain: 'domain',
  subdomain: 'subdomain',
  schema: 'schema',
  project: 'project',
  screen: 'screen',
  term: 'term',
  ambiguity: 'ambiguity',
  serviceLevelIndicator: 'service_level_indicator',
  serviceLevelObjective: 'service_level_objective',
  alertPolicy: 'alert_policy',
  alertNotificationTarget: 'alert_notification_target',
  typeLibrary: 'type_library',
};

function isOverviewMode(value: string | null): value is OverviewMode {
  return value !== null && (OVERVIEW_MODES as readonly string[]).includes(value);
}

function readUrlDomains(): string[] {
  const p = new URLSearchParams(window.location.search);
  const raw = p.get('domain') ?? '';
  return raw.split(',').filter(Boolean);
}

function readUrlMode(): OverviewMode | undefined {
  const raw = new URLSearchParams(window.location.search).get('mode');
  return isOverviewMode(raw) ? raw : undefined;
}

function persistDomains(slugs: string[]) {
  const p = new URLSearchParams(window.location.search);
  if (slugs.length === 0) p.delete('domain');
  else p.set('domain', slugs.join(','));
  const qs = p.toString();
  window.history.replaceState(null, '', `${window.location.pathname}${qs ? `?${qs}` : ''}`);
  try {
    if (slugs.length === 0) window.localStorage.removeItem(STORAGE_KEY);
    else window.localStorage.setItem(STORAGE_KEY, slugs.join(','));
  } catch {
    // localStorage may be unavailable; URL is the source of truth.
  }
}

function persistMode(mode: OverviewMode) {
  const p = new URLSearchParams(window.location.search);
  if (mode === 'chart') p.delete('mode');
  else p.set('mode', mode);
  const qs = p.toString();
  window.history.replaceState(null, '', `${window.location.pathname}${qs ? `?${qs}` : ''}`);
  try {
    if (mode === 'chart') window.localStorage.removeItem(MODE_STORAGE_KEY);
    else window.localStorage.setItem(MODE_STORAGE_KEY, mode);
  } catch {
    // localStorage may be unavailable; URL is the source of truth.
  }
}

export function OverviewPage() {
  const [chart, setChart] = useState<Model>();
  const [eventModels, setEventModels] = useState<Entity[]>([]);
  const [overviewError, setOverviewError] = useState<string>();
  const [searchError, setSearchError] = useState<string>();
  const [searchQuery, setSearchQuery] = useState('');
  const [searching, setSearching] = useState(false);
  const [searchResults, setSearchResults] = useState<Entity[]>([]);
  const [chartFocusKey, setChartFocusKey] = useState<string>();
  const [selectedDomains, setSelectedDomains] = useState<string[]>(() => {
    const fromUrl = readUrlDomains();
    if (fromUrl.length > 0) return fromUrl;
    try {
      const stored = window.localStorage.getItem(STORAGE_KEY) ?? '';
      return stored.split(',').filter(Boolean);
    } catch {
      return [];
    }
  });
  const [mode, setMode] = useState<OverviewMode>(() => {
    const fromUrl = readUrlMode();
    if (fromUrl) return fromUrl;
    try {
      const stored = window.localStorage.getItem(MODE_STORAGE_KEY);
      return isOverviewMode(stored) ? stored : 'chart';
    } catch {
      return 'chart';
    }
  });

  const selectMode = (next: OverviewMode) => {
    setMode(next);
    persistMode(next);
  };

  const branch = readBranchFromUrl();

  // OverviewPage never re-reads ?branch= after mount (it isn't in the nuqs
  // tree and has no owning writer of its own), which matches its existing
  // once-per-mount idiom for ?domain=/?mode= (persistDomains/persistMode
  // patch the URL without triggering a refetch either).
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally mount-only, see above
  useEffect(() => {
    const ctrl = new AbortController();
    api
      .overview({ signal: ctrl.signal, branch })
      .then((res) => {
        if (ctrl.signal.aborted) return;
        const all = buildModel(res.entities ?? []);
        setEventModels(all.entities.filter((e) => e.kind === 'eventModel'));
        setChart(all);
      })
      .catch((e) => {
        if (ctrl.signal.aborted) return;
        setOverviewError(e instanceof Error ? e.message : String(e));
      });
    return () => ctrl.abort();
  }, []);

  useEffect(() => {
    const q = searchQuery.trim();
    if (q.length < 2) {
      setSearchResults([]);
      setSearching(false);
      return;
    }
    const ctrl = new AbortController();
    const timer = window.setTimeout(() => {
      setSearching(true);
      api
        .search(q, undefined, { signal: ctrl.signal })
        .then((res) => {
          if (ctrl.signal.aborted) return;
          const entities = (res.results ?? []).flatMap((hit) => (hit.entity ? [hit.entity] : []));
          setSearchResults(buildModel(entities).entities);
          setSearching(false);
        })
        .catch((e) => {
          if (ctrl.signal.aborted) return;
          setSearchError(e instanceof Error ? e.message : String(e));
          setSearching(false);
        });
    }, 200);
    return () => {
      window.clearTimeout(timer);
      ctrl.abort();
    };
  }, [searchQuery]);

  // Map EventModel namespace → owning Domain slug. EventModel lives in a
  // BoundedContext namespace; the BC realizes subdomain(s); each subdomain
  // belongs to a domain. Walk that chain once per render.
  const domainOfModel = useMemo(() => {
    if (!chart) return new Map<string, string | undefined>();
    const byNs = new Map<string, Entity>();
    for (const e of chart.entities) {
      if (e.kind === 'boundedContext') byNs.set(e.id.namespace, e);
    }
    const subdomainBySlug = new Map<string, Entity>();
    for (const e of chart.entities) {
      if (e.kind === 'subdomain') subdomainBySlug.set(`${e.id.namespace}/${e.id.slug}`, e);
    }
    const out = new Map<string, string | undefined>();
    for (const em of eventModels) {
      const bc = byNs.get(em.id.namespace);
      if (!bc) {
        out.set(em.key, undefined);
        continue;
      }
      // Pick the first realized subdomain's domain, almost always one;
      // Decision #29 says >1 is a monolith confessing.
      const sdId = realizesOf(bc)[0];
      if (!sdId) {
        out.set(em.key, undefined);
        continue;
      }
      const sd = subdomainBySlug.get(`${sdId.namespace}/${sdId.slug}`);
      if (!sd) {
        out.set(em.key, undefined);
        continue;
      }
      const dom = domainOf(sd);
      out.set(em.key, dom?.slug);
    }
    return out;
  }, [chart, eventModels]);

  const availableDomains = useMemo(() => {
    if (!chart) return [] as { slug: string; title: string; classification?: string }[];
    return chart.entities
      .filter((e) => e.kind === 'domain')
      .map((e) => ({
        slug: e.id.slug,
        title: e.title,
        classification: classificationOf(e),
      }))
      .sort((a, b) => a.title.localeCompare(b.title));
  }, [chart]);

  const homeIndex = useMemo(() => {
    const homes = new Map<string, Entity[]>();
    for (const em of eventModels) {
      homes.set(`${em.kind}:${em.id.namespace}/${em.id.slug}`, [em]);
      const members = Array.isArray(em.raw.members) ? em.raw.members : [];
      for (const member of members) {
        const rec = member as Record<string, unknown>;
        const kind = wireKind(rec.kind);
        const id = asId(rec.id);
        if (!kind || !id.namespace || !id.slug) continue;
        const key = `${kind}:${id.namespace}/${id.slug}`;
        const list = homes.get(key) ?? [];
        list.push(em);
        homes.set(key, list);
      }
    }
    return homes;
  }, [eventModels]);

  const homesFor = (entity: Entity): Entity[] =>
    homeIndex.get(`${entity.kind}:${entity.id.namespace}/${entity.id.slug}`) ?? [];

  // Validate hydrated slugs against the live domain list once it loads.
  // Persisted state may contain slugs that no longer exist (renamed,
  // deleted, or hand-crafted in localStorage); silently drop them so a
  // stale filter cannot survive across sessions or be smuggled in via
  // localStorage edits.
  useEffect(() => {
    if (!chart || availableDomains.length === 0) return;
    const allowed = new Set(availableDomains.map((d) => d.slug));
    setSelectedDomains((prev) => {
      const next = prev.filter((s) => allowed.has(s));
      if (next.length === prev.length) return prev;
      persistDomains(next);
      return next;
    });
  }, [chart, availableDomains]);

  const toggleDomain = (slug: string) => {
    setSelectedDomains((prev) => {
      const next = prev.includes(slug) ? prev.filter((s) => s !== slug) : [...prev, slug];
      persistDomains(next);
      return next;
    });
  };

  const visibleEventModels = useMemo(() => {
    if (selectedDomains.length === 0) return eventModels;
    return eventModels.filter((em) => {
      const dom = domainOfModel.get(em.key);
      return dom ? selectedDomains.includes(dom) : false;
    });
  }, [eventModels, selectedDomains, domainOfModel]);

  const visibleChart = useMemo(() => {
    if (!chart || selectedDomains.length === 0) return chart;
    const wanted = new Set(selectedDomains);
    const subs = chart.entities.filter((e) => {
      if (e.kind !== 'subdomain') return false;
      const dom = domainOf(e);
      return dom ? wanted.has(dom.slug) : false;
    });
    const subKeys = new Set(subs.map((s) => `${s.id.namespace}/${s.id.slug}`));
    const contexts = chart.entities.filter(
      (e) => e.kind === 'boundedContext' && realizesOf(e).some((r) => subKeys.has(`${r.namespace}/${r.slug}`)),
    );
    const domains = chart.entities.filter((e) => e.kind === 'domain' && wanted.has(e.id.slug));
    const kept = [...domains, ...subs, ...contexts];
    const byKind = new Map<import('@/lib/model').EntityKind, Entity[]>();
    for (const e of kept) {
      const list = byKind.get(e.kind) ?? [];
      list.push(e);
      byKind.set(e.kind, list);
    }
    return {
      entities: kept,
      byKey: new Map(kept.map((e) => [e.key, e])),
      byKind,
      slices: [],
      storyboards: [],
      continuations: [],
      swimlanes: [],
      tracking: new Map(),
      namespaces: [...new Set(kept.map((e) => e.id.namespace))],
    } satisfies Model;
  }, [chart, selectedDomains]);

  // Memoize so `mode` / `error` / unrelated state toggles don't rerun the
  // O(n) group + sort. Only changes to the actual event-model set should
  // trigger recompute.
  const grouped = useMemo(() => {
    const map = new Map<string, Entity[]>();
    for (const em of visibleEventModels) {
      const list = map.get(em.id.namespace) ?? [];
      list.push(em);
      map.set(em.id.namespace, list);
    }
    return map;
  }, [visibleEventModels]);
  const sortedNamespaces = useMemo(() => [...grouped.keys()].sort(), [grouped]);
  const chartFocusEntity = chartFocusKey && visibleChart ? visibleChart.byKey.get(chartFocusKey) : undefined;

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <header className="border-b border-border bg-card px-6 py-4">
        <div className="flex flex-wrap items-baseline gap-3">
          <h1 className="text-lg font-semibold">Overview</h1>
          <p className="text-xs text-muted-foreground">
            The map of everything. Pick an event model to walk a flow; click a domain or context to read its charter.
          </p>
          <div className="ml-auto inline-flex overflow-hidden rounded-md border border-border text-xs">
            {OVERVIEW_MODES.map((m) => (
              <button
                key={m}
                type="button"
                onClick={() => selectMode(m)}
                aria-pressed={mode === m}
                className={cn(
                  'px-3 py-1 font-medium transition-colors',
                  mode === m
                    ? 'bg-foreground text-background'
                    : 'bg-card text-foreground hover:bg-accent hover:text-accent-foreground',
                )}
              >
                {OVERVIEW_MODE_LABELS[m]}
              </button>
            ))}
          </div>
        </div>
        <div className="mt-3 flex max-w-xl items-center gap-2 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm">
          <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
          <input
            aria-label="Search models and entities"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search models and entities"
            className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
          {searching ? <span className="text-[10px] text-muted-foreground">Searching</span> : null}
        </div>
        {availableDomains.length > 1 ? (
          <div className="mt-3 flex flex-wrap items-center gap-1.5 text-xs">
            <span className="mr-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
              Domains
            </span>
            {availableDomains.map((d) => {
              const active = selectedDomains.includes(d.slug);
              return (
                <button
                  key={d.slug}
                  type="button"
                  onClick={() => toggleDomain(d.slug)}
                  className={cn(
                    'rounded-full border px-2.5 py-1 font-medium transition-colors',
                    active
                      ? 'border-indigo-500 bg-indigo-500 text-white'
                      : 'border-border bg-card text-foreground hover:border-indigo-300 hover:bg-indigo-50/60',
                  )}
                >
                  {d.title}
                  {d.classification ? (
                    <span
                      className={cn(
                        'ml-1.5 text-[9px] font-bold uppercase',
                        active ? 'text-indigo-100' : 'text-muted-foreground',
                      )}
                    >
                      {d.classification}
                    </span>
                  ) : null}
                </button>
              );
            })}
            {selectedDomains.length > 0 ? (
              <button
                type="button"
                onClick={() => {
                  setSelectedDomains([]);
                  persistDomains([]);
                }}
                className="ml-1 text-[10px] text-muted-foreground underline-offset-2 hover:underline"
              >
                clear
              </button>
            ) : (
              <span className="ml-1 text-[10px] text-muted-foreground">showing all</span>
            )}
          </div>
        ) : null}
      </header>
      {overviewError ? (
        <div role="alert" aria-live="assertive" className="border-b border-border px-6 py-2 text-xs text-red-600">
          {overviewError}
        </div>
      ) : null}
      {mode === 'chart' ? (
        <section className="border-b border-border" style={{ height: '40%' }}>
          {visibleChart ? (
            <ReactFlowProvider>
              <ErrorBoundary>
                <DomainChartBoard
                  model={visibleChart}
                  selections={(chartFocusKey ? [{ type: 'entity', key: chartFocusKey }] : []) as Selection[]}
                  onSelect={(key) => setChartFocusKey(key)}
                />
              </ErrorBoundary>
            </ReactFlowProvider>
          ) : (
            <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
              Loading the map…
            </div>
          )}
        </section>
      ) : mode === 'contexts' ? (
        <section className="border-b border-border" style={{ height: '40%' }}>
          {chart ? (
            <ReactFlowProvider>
              <ErrorBoundary>
                <ContextMapBoard model={chart} selections={[] as Selection[]} onSelect={() => {}} />
              </ErrorBoundary>
            </ReactFlowProvider>
          ) : (
            <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
              Loading the context map…
            </div>
          )}
        </section>
      ) : null}
      {chartFocusEntity && mode === 'chart' ? (
        <div className="border-b border-border bg-card px-6 py-3">
          <div className="text-sm font-semibold">{chartFocusEntity.title}</div>
          {chartFocusEntity.doc ? (
            <p className="mt-1 text-xs text-muted-foreground">{chartFocusEntity.doc}</p>
          ) : (
            <p className="mt-1 text-xs text-muted-foreground">No charter documented yet.</p>
          )}
        </div>
      ) : null}
      <section className="flex-1 overflow-y-auto p-6">
        {searchQuery.trim().length >= 2 ? (
          <div className="mb-6">
            <h2 className="mb-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              Search Results ({searchResults.length})
            </h2>
            {searchError ? (
              <div role="alert" aria-live="assertive" className="text-xs text-red-600">
                {searchError}
              </div>
            ) : null}
            {searchResults.length > 0 ? (
              <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
                {searchResults.map((entity) => {
                  const homes = homesFor(entity);
                  const href =
                    homes[0] && KIND_SHORT[entity.kind]
                      ? withBranchQuery(
                          `/em/${encodeURIComponent(homes[0].id.namespace)}/${encodeURIComponent(homes[0].id.slug)}?focus=${encodeURIComponent(
                            `${KIND_SHORT[entity.kind]}:${entity.id.namespace}/${entity.id.slug}`,
                          )}`,
                          branch,
                        )
                      : undefined;
                  const body = (
                    <>
                      <div className="flex items-center gap-2">
                        <span className="truncate font-semibold">{entity.title}</span>
                        <span className="ml-auto shrink-0 rounded bg-zinc-100 px-1.5 py-0.5 text-[9px] font-bold uppercase text-zinc-600">
                          {entity.kind}
                        </span>
                      </div>
                      <div className="mt-0.5 truncate font-mono text-[10px] text-muted-foreground">
                        {entity.id.namespace}/{entity.id.slug}@v{entity.id.version}
                      </div>
                      <div className="mt-1 truncate text-[10px] text-muted-foreground">
                        {homes.length > 0 ? `in ${homes.map((home) => home.title).join(', ')}` : entity.doc}
                      </div>
                    </>
                  );
                  return href ? (
                    <a
                      key={entity.key}
                      href={href}
                      className="block rounded-lg border border-border bg-card px-3 py-2 text-sm hover:border-indigo-300 hover:bg-indigo-50/40"
                    >
                      {body}
                    </a>
                  ) : (
                    <div key={entity.key} className="rounded-lg border border-border bg-card px-3 py-2 text-sm">
                      {body}
                    </div>
                  );
                })}
              </div>
            ) : (
              <div className="text-xs text-muted-foreground">No matching entities.</div>
            )}
          </div>
        ) : null}
        <h2 className="mb-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Event Models ({visibleEventModels.length}
          {selectedDomains.length > 0 ? ` of ${eventModels.length}` : ''})
        </h2>
        <div className="space-y-6">
          {sortedNamespaces.map((ns) => {
            const list = (grouped.get(ns) ?? []).sort((a, b) => a.title.localeCompare(b.title));
            return (
              <div key={ns}>
                <h3 className="mb-2 font-mono text-[11px] font-semibold uppercase text-muted-foreground">
                  {ns}
                  <span className="ml-2 text-muted-foreground">·</span>
                  <span className="ml-2 font-sans normal-case">
                    {list.length} model{list.length === 1 ? '' : 's'}
                  </span>
                </h3>
                <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
                  {list.map((em) => {
                    const members = Array.isArray(em.raw.members) ? em.raw.members.length : 0;
                    return (
                      <a
                        key={em.key}
                        href={withBranchQuery(
                          `/em/${encodeURIComponent(em.id.namespace)}/${encodeURIComponent(em.id.slug)}`,
                          branch,
                        )}
                        className={cn(
                          'block rounded-lg border border-border bg-card px-3 py-2 text-sm hover:border-indigo-300 hover:bg-indigo-50/40',
                        )}
                      >
                        <div className="truncate font-semibold">{em.title}</div>
                        <div className="mt-0.5 font-mono text-[10px] text-muted-foreground">
                          {em.id.namespace}/{em.id.slug}@v{em.id.version}
                        </div>
                        <div className="mt-1 text-[10px] text-muted-foreground">
                          {members} member{members === 1 ? '' : 's'}
                          {em.doc ? ` · ${em.doc.slice(0, 80)}${em.doc.length > 80 ? '…' : ''}` : ''}
                        </div>
                      </a>
                    );
                  })}
                </div>
              </div>
            );
          })}
          {sortedNamespaces.length === 0 ? (
            <div className="text-xs text-muted-foreground">
              No event models match the selected domain{selectedDomains.length === 1 ? '' : 's'}.
            </div>
          ) : null}
        </div>
      </section>
    </div>
  );
}

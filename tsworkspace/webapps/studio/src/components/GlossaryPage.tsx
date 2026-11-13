import { BookOpen } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { GlossaryComparisonPanel, GlossaryFailure, GLOSSARY_SELECT_CLASS } from '@/components/GlossaryReviewParts';
import { GlossaryTermPanel } from '@/components/GlossaryTermPanel';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { buttonVariants } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { api, getGlossaryTerm, listGlossaryEntries } from '@/lib/api';
import { readBranchFromUrl, withBranchQuery } from '@/lib/branch';
import {
  exactGlossaryId,
  glossaryIdLabel,
  glossaryStateLabel,
  sameGlossaryId,
  type GlossarySelection,
} from '@/lib/glossary';
import type { EntityId } from '@/lib/model';
import { useGlossaryRead } from '@/lib/useGlossaryRead';
import { isSafeNamespace } from '../../shared/safe-namespace.mjs';

interface LocationSelection {
  namespace: string;
  audience: string;
  includeUnreviewed: boolean;
  term?: EntityId;
  branch?: string;
  historical: boolean;
  error?: string;
}
function readLocation(): LocationSelection {
  const params = new URLSearchParams(window.location.search);
  const rawNamespace = params.get('namespace') ?? '';
  const namespace = rawNamespace && isSafeNamespace(rawNamespace) ? rawNamespace : '';
  const hasTerm = params.has('slug') || params.has('version');
  const term = exactGlossaryId({ namespace, slug: params.get('slug'), version: params.get('version') });
  return {
    namespace,
    audience: params.get('audience') ?? '',
    includeUnreviewed: params.get('unreviewed') !== 'false',
    term,
    branch: readBranchFromUrl(),
    historical: params.has('asOf'),
    error:
      (rawNamespace && !namespace) || (hasTerm && !term)
        ? 'The glossary link does not identify a valid context and exact term version.'
        : undefined,
  };
}

export function GlossaryPage() {
  const [location, setLocation] = useState(readLocation);
  const [pageToken, setPageToken] = useState<string>();
  const [previousPages, setPreviousPages] = useState<(string | undefined)[]>([]);
  useEffect(() => {
    const onPop = () => {
      setLocation(readLocation());
      setPageToken(undefined);
      setPreviousPages([]);
    };
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, []);
  const { namespace, audience, includeUnreviewed, branch, historical, term } = location;
  const selection = useMemo<GlossarySelection>(
    () => ({ namespaces: [namespace], ...(audience ? { audience } : {}), includeUnreviewed }),
    [namespace, audience, includeUnreviewed],
  );
  const loadNamespaces = useCallback((signal: AbortSignal) => api.namespaces({ signal, branch }), [branch]);
  const directory = useGlossaryRead(historical ? null : JSON.stringify(['namespaces', branch]), loadNamespaces);
  const loadCatalog = useCallback(
    (signal: AbortSignal) =>
      listGlossaryEntries({ selection, pageSize: 100, ...(pageToken ? { pageToken } : {}) }, { signal, branch }),
    [selection, pageToken, branch],
  );
  const catalog = useGlossaryRead(
    namespace && !historical ? JSON.stringify([selection, pageToken, branch]) : null,
    loadCatalog,
  );
  const loadDetail = useCallback(
    (signal: AbortSignal) => getGlossaryTerm({ term: { id: term } }, { signal, branch }),
    [term, branch],
  );
  const detail = useGlossaryRead(term && !historical ? JSON.stringify([term, branch]) : null, loadDetail);
  const namespaces = directory.state.status === 'ready' ? directory.state.value.namespaces : [];
  const labels = directory.state.status === 'ready' ? (directory.state.value.labels ?? {}) : {};
  const entries = catalog.state.status === 'ready' ? (catalog.state.value.entries ?? []) : [];
  const updateLocation = (patch: Partial<LocationSelection>, selected?: EntityId) => {
    const next = { ...location, ...patch, term: selected, error: undefined };
    const params = new URLSearchParams(window.location.search);
    for (const key of ['namespace', 'audience', 'unreviewed', 'slug', 'version']) params.delete(key);
    if (next.namespace) params.set('namespace', next.namespace);
    if (next.audience) params.set('audience', next.audience);
    if (!next.includeUnreviewed) params.set('unreviewed', 'false');
    if (selected) {
      params.set('slug', selected.slug);
      params.set('version', selected.version);
    }
    window.history.replaceState(null, '', `/glossary${params.size ? `?${params}` : ''}`);
    setLocation(next);
    if (!selected) {
      setPageToken(undefined);
      setPreviousPages([]);
    }
  };
  return (
    <div className="flex min-h-screen flex-col bg-background">
      <header className="flex flex-wrap items-center gap-4 border-b border-border bg-card px-6 py-4">
        <BookOpen className="size-5 text-muted-foreground" aria-hidden="true" />
        <div className="flex flex-1 flex-col gap-1">
          <h1 className="text-lg font-semibold">Glossary</h1>
          <p className="text-xs text-muted-foreground">Vocabulary, evidence and review within each bounded context.</p>
        </div>
        <a href={withBranchQuery('/', branch)} className={buttonVariants({ variant: 'outline', size: 'sm' })}>
          Overview
        </a>
      </header>
      <main className="flex flex-1 flex-col gap-6 p-6">
        {historical ? (
          <output>
            Historical glossary inspection is unavailable. Return to the live glossary to inspect current evidence.
          </output>
        ) : (
          <>
            {location.error ? <p role="alert">{location.error}</p> : null}
            <div className="flex flex-wrap items-end gap-4">
              <label className="flex min-w-52 flex-col gap-2 text-sm font-medium">
                Bounded context
                <select
                  className={GLOSSARY_SELECT_CLASS}
                  value={namespace}
                  onChange={(event) => updateLocation({ namespace: event.target.value })}
                >
                  <option value="">Choose a bounded context</option>
                  {namespace && !namespaces.includes(namespace) ? <option value={namespace}>{namespace}</option> : null}
                  {namespaces.map((item) => (
                    <option key={item} value={item}>
                      {labels[item] ?? item}
                    </option>
                  ))}
                </select>
              </label>
              <label htmlFor="glossary-audience" className="flex min-w-48 flex-col gap-2 text-sm">
                Audience
                <Input
                  id="glossary-audience"
                  placeholder="Any audience"
                  value={audience}
                  onChange={(event) => updateLocation({ audience: event.target.value })}
                />
              </label>
              <label className="flex h-9 items-center gap-2 text-xs">
                <input
                  type="checkbox"
                  checked={includeUnreviewed}
                  onChange={(event) => updateLocation({ includeUnreviewed: event.target.checked })}
                />
                Include unreviewed candidates
              </label>
              {branch ? <Badge variant="outline">Branch: {branch}</Badge> : null}
            </div>
            {directory.state.status === 'failed' ? (
              <GlossaryFailure message={directory.state.error} retry={directory.retry} />
            ) : null}
            {!namespace ? (
              <p className="text-sm text-muted-foreground">Choose a bounded context to inspect its vocabulary.</p>
            ) : (
              <div className="grid items-start gap-6 lg:grid-cols-[minmax(18rem,1fr)_minmax(0,2fr)]">
                <div className="flex min-w-0 flex-col gap-4">
                  <section
                    aria-label="Glossary entries"
                    className="flex flex-col gap-3 rounded-lg border border-border bg-card p-4"
                  >
                    <div className="flex items-center justify-between gap-3">
                      <h2 className="text-sm font-semibold">Vocabulary</h2>
                      <Button variant="outline" size="sm" onClick={catalog.retry}>
                        Reload
                      </Button>
                    </div>
                    {catalog.state.status === 'loading' ? (
                      <output className="text-xs">
                        Loading glossary…
                      </output>
                    ) : null}
                    {catalog.state.status === 'failed' ? (
                      <GlossaryFailure message={catalog.state.error} retry={catalog.retry} />
                    ) : null}
                    {catalog.state.status === 'ready' ? (
                      <>
                        {catalog.state.value.incomplete ? (
                          <output className="text-xs text-amber-800">
                            The glossary scan is incomplete. Missing entries do not establish absence.
                          </output>
                        ) : null}
                        {entries.length === 0 ? (
                          <p className="text-xs text-muted-foreground">
                            {catalog.state.value.incomplete
                              ? 'No entries returned in this incomplete scan.'
                              : 'No glossary entries.'}
                          </p>
                        ) : (
                          <ul className="flex flex-col gap-2">
                            {entries.map((entry) => {
                              const id = exactGlossaryId(entry.term?.id);
                              return (
                                <li key={id ? glossaryIdLabel(id) : JSON.stringify(entry)}>
                                  <button
                                    type="button"
                                    disabled={!id}
                                    aria-pressed={sameGlossaryId(id, term)}
                                    onClick={() => {
                                      if (id) updateLocation({}, id);
                                    }}
                                    className="flex w-full flex-col gap-2 rounded-md border border-border p-3 text-left hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring aria-pressed:border-lime-600 aria-pressed:bg-lime-50"
                                  >
                                    <span className="text-sm font-medium">
                                      {entry.title || id?.slug || 'Entry unavailable'}
                                    </span>
                                    <span className="break-all font-mono text-[10px] text-muted-foreground">
                                      {id ? glossaryIdLabel(id) : 'Exact identity unavailable'}
                                    </span>
                                    <span className="flex flex-wrap gap-2">
                                      <Badge variant="secondary">{glossaryStateLabel(entry.state)}</Badge>
                                      {entry.published === true ? <Badge variant="outline">Published</Badge> : null}
                                      {entry.audience ? <Badge variant="outline">{entry.audience}</Badge> : null}
                                    </span>
                                  </button>
                                </li>
                              );
                            })}
                          </ul>
                        )}
                        <div className="flex flex-wrap gap-2">
                          {previousPages.length ? (
                            <Button
                              variant="outline"
                              size="sm"
                              onClick={() => {
                                setPageToken(previousPages.at(-1));
                                setPreviousPages((pages) => pages.slice(0, -1));
                              }}
                            >
                              Previous page
                            </Button>
                          ) : null}
                          {catalog.state.value.nextPageToken ? (
                            <Button
                              variant="outline"
                              size="sm"
                              onClick={() => {
                                if (catalog.state.status === 'ready') {
                                  setPreviousPages((pages) => [...pages, pageToken]);
                                  setPageToken(catalog.state.value.nextPageToken);
                                }
                              }}
                            >
                              Next page
                            </Button>
                          ) : null}
                        </div>
                      </>
                    ) : null}
                  </section>
                  <GlossaryComparisonPanel
                    key={JSON.stringify(selection)}
                    selection={selection}
                    namespaces={namespaces}
                    labels={labels}
                    branch={branch}
                  />
                </div>
                <div className="flex min-w-0 flex-col gap-4">
                  {detail.state.status === 'loading' ? (
                    <output className="text-sm">
                      Loading exact term…
                    </output>
                  ) : null}
                  {detail.state.status === 'failed' ? (
                    <GlossaryFailure message={detail.state.error} retry={detail.retry} />
                  ) : null}
                  {detail.state.status === 'ready' && term ? (
                    <GlossaryTermPanel
                      key={JSON.stringify([term, detail.state.value.etag])}
                      detail={detail.state.value}
                      selected={term}
                      selection={selection}
                      branch={branch}
                    />
                  ) : !term ? (
                    <p className="rounded-lg border border-dashed border-border p-6 text-sm text-muted-foreground">
                      Select a term to inspect its definition, evidence and review.
                    </p>
                  ) : null}
                </div>
              </div>
            )}
          </>
        )}
      </main>
    </div>
  );
}

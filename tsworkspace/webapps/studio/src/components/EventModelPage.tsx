// EventModelPage: landing for /em/<namespace>/<slug>. Loads exactly one
// EventModel's member set (plus halo) via the dedicated endpoint and hands
// it to ModelShell. The board only has to render this model, so it loads
// fast even when the store holds 1500+ entities (Decision: the unit of
// viewing is the EventModel, not the store).
import { useCallback, useEffect, useRef, useState } from 'react';
import { ModelShell } from '@/components/ModelShell';
import { api } from '@/lib/api';
import { readBranchFromUrl, withBranchQuery } from '@/lib/branch';
import { buildIssueIndex, EMPTY_ISSUE_INDEX, type IssueIndex } from '@/lib/issues';
import { buildModel, type Model } from '@/lib/model';

export function EventModelPage({ namespace, slug }: { namespace: string; slug: string }) {
  const [model, setModel] = useState<Model>();
  const [issues, setIssues] = useState<IssueIndex>(EMPTY_ISSUE_INDEX);
  const [error, setError] = useState<string>();

  // Tied to an `AbortController` so unmount mid-fetch actually aborts
  // the HTTP request, not just hides its result. The 30 s default
  // timeout in `api.ts` would otherwise hold the connection open.
  // `load` is also handed to ModelShell as the realtime refetch, so a
  // store change reloads this model in place; each call aborts the
  // previous in-flight fetch.
  // Branch is read inside `load` (not closed over at render) so a
  // ?branch= change from ModelShell/BranchBar is honored on the next
  // refetch even when this page has not remounted.
  const ctrlRef = useRef<AbortController | undefined>(undefined);
  const load = useCallback(() => {
    ctrlRef.current?.abort();
    const ctrl = new AbortController();
    ctrlRef.current = ctrl;
    const branch = readBranchFromUrl();
    api
      .eventModel(namespace, slug, { signal: ctrl.signal, branch })
      .then((res) => {
        if (ctrl.signal.aborted) return;
        setModel(buildModel(res.entities ?? []));
        setError(undefined);
      })
      .catch((e) => {
        if (ctrl.signal.aborted) return;
        setError(e instanceof Error ? e.message : String(e));
      });
    // Validation rides alongside rather than gating the board: findings are
    // useful the moment they arrive, and a validator that is slow or down
    // must not stop the model from rendering. A failure is carried into the
    // index instead of dropped, so the panel can say "did not run" rather
    // than showing the same empty section a clean model would.
    api
      .validate({ namespace, slug }, { signal: ctrl.signal, branch })
      .then((res) => {
        if (ctrl.signal.aborted) return;
        setIssues(buildIssueIndex(res.issues));
      })
      .catch((e) => {
        if (ctrl.signal.aborted) return;
        setIssues(buildIssueIndex(undefined, e instanceof Error ? e.message : String(e)));
      });
  }, [namespace, slug]);

  useEffect(() => {
    load();
    return () => {
      ctrlRef.current?.abort();
    };
  }, [load]);

  // A transient refetch failure must not replace an already-rendered
  // board with the error page; the error screen is for the initial load.
  const branch = readBranchFromUrl();
  if (error && !model) {
    return (
      <div className="flex h-screen items-center justify-center p-6">
        <div className="max-w-md rounded-lg border border-red-300 bg-red-50 px-4 py-3 text-sm text-red-700">
          <div className="font-semibold">Could not load event model</div>
          <div className="mt-1 font-mono text-xs">
            {namespace}/{slug}
          </div>
          <div className="mt-2">{error}</div>
          <a href={withBranchQuery('/', branch)} className="mt-3 inline-block text-xs underline">
            ← Back to Overview
          </a>
        </div>
      </div>
    );
  }

  if (!model) {
    return (
      <div className="flex h-screen items-center justify-center text-sm text-muted-foreground">
        Loading {namespace}/{slug}…
      </div>
    );
  }

  const em = model.entities.find((e) => e.kind === 'eventModel' && e.id.namespace === namespace && e.id.slug === slug);
  return (
    <ModelShell
      model={model}
      scopedTitle={em?.title ?? `${namespace}/${slug}`}
      backHref={withBranchQuery('/', branch)}
      issues={issues}
      onRefetch={load}
    />
  );
}

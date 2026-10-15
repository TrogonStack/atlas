// Namespace history drawer: the changesets that touched one namespace, newest
// first. Titled by namespace, not model, because a namespace can hold more
// than one model and the server filters by namespace only.
import { X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { api, type ChangesetOpView, type ChangesetView } from '@/lib/api';
import { entityKey, type Model, wireKind } from '@/lib/model';
import { cn } from '@/lib/utils';

function formatAt(at: string): string {
  const d = new Date(at);
  return Number.isNaN(d.getTime()) ? at : d.toLocaleString();
}

function opLabel(op: ChangesetOpView): string {
  const kind = op.entity.kind.replace('ENTITY_KIND_', '').toLowerCase();
  const verb = op.kind === 'KIND_DELETED' ? 'deleted' : 'put';
  return `${verb} ${kind} ${op.entity.namespace}/${op.entity.slug}@${op.entity.version}`;
}

// Only an entity on this board can open the Inspector; deletes and entities of
// other models in the namespace have nothing to select, so they render as text.
function opModelKey(model: Model, op: ChangesetOpView): string | undefined {
  const kind = wireKind(op.entity.kind);
  if (!kind) return undefined;
  const { namespace, slug, version } = op.entity;
  const key = entityKey(kind, { namespace, slug, version });
  return model.byKey.has(key) ? key : undefined;
}

// Mounted only while open, so the list is read on demand rather than on every board load.
export function HistoryPanel({
  namespace,
  branch,
  model,
  onClose,
  onSelect,
  onViewAsOf,
}: {
  namespace: string;
  /** Previewed branch: lists that branch's changesets instead of every changeset. */
  branch?: string;
  model: Model;
  onClose: () => void;
  onSelect: (key: string) => void;
  /** Opens the board as of a baseline changeset. Omit where the board cannot rewind. */
  onViewAsOf?: (changesetId: string) => void;
}) {
  const [changesets, setChangesets] = useState<ChangesetView[]>([]);
  const [nextPageToken, setNextPageToken] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const [expanded, setExpanded] = useState<string>();
  const abortRef = useRef<AbortController>(undefined);

  const load = async (pageToken?: string) => {
    abortRef.current?.abort();
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    setLoading(true);
    setError(undefined);
    try {
      const page = await api.changesets(namespace, pageToken, { signal: ctrl.signal, branch });
      setChangesets((prev) => (pageToken ? [...prev, ...page.changesets] : page.changesets));
      setNextPageToken(page.nextPageToken);
    } catch (e) {
      if (ctrl.signal.aborted) return;
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (!ctrl.signal.aborted) setLoading(false);
    }
  };

  // ModelShell keys this panel by namespace and branch, so either change
  // starts again from the newest page.
  useEffect(() => {
    void load();
    return () => abortRef.current?.abort();
    // oxlint-disable-next-line react-hooks/exhaustive-deps -- mount-only, see above
  }, []);

  return (
    <aside aria-label="Namespace history" className="flex h-full w-[24rem] flex-col border-l border-border bg-card">
      <div className="flex items-start gap-2 p-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate font-mono text-[11px] text-muted-foreground">{namespace}</span>
            {branch ? (
              <Badge variant="outline" className="border-amber-400 bg-amber-100 font-mono text-amber-900">
                {branch}
              </Badge>
            ) : null}
          </div>
          <h2 className="mt-2 text-base font-semibold leading-tight">Namespace history</h2>
          <p className="mt-1 text-[10px] text-muted-foreground">
            Every recorded change to this namespace, which can hold more than this model.
          </p>
        </div>
        <Button variant="ghost" size="icon" className="ml-auto" aria-label="Close namespace history" onClick={onClose}>
          <X />
        </Button>
      </div>
      <Separator />
      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto p-3">
        {changesets.length === 0 && !loading && !error ? (
          <p className="text-[11px] italic text-muted-foreground">No recorded changesets for this namespace.</p>
        ) : null}
        <ul className="space-y-1">
          {changesets.map((c) => (
            <li key={c.id} className="rounded-md border border-border">
              <button
                type="button"
                title={c.message || undefined}
                aria-expanded={expanded === c.id}
                onClick={() => setExpanded((cur) => (cur === c.id ? undefined : c.id))}
                className={cn(
                  'flex w-full flex-wrap items-center gap-x-2 px-2 py-1 text-left text-[10px] hover:bg-accent',
                  expanded === c.id && 'bg-accent',
                )}
              >
                <span className="text-muted-foreground">{formatAt(c.at)}</span>
                <span className="truncate">{c.author}</span>
                <span className="font-semibold">{c.rpc}</span>
                <span className="text-muted-foreground">{c.ops.length === 1 ? '1 op' : `${c.ops.length} ops`}</span>
                {c.branch ? (
                  <span className="ml-auto rounded bg-amber-100 px-1 font-mono text-amber-900">{c.branch}</span>
                ) : null}
              </button>
              {expanded === c.id ? (
                <ul className="space-y-0.5 border-t border-border p-2 font-mono text-[10px]">
                  {c.ops.map((op) => {
                    const key = opModelKey(model, op);
                    return (
                      <li key={opLabel(op)}>
                        {key ? (
                          <button
                            type="button"
                            onClick={() => onSelect(key)}
                            className="w-full truncate rounded px-1 text-left text-primary hover:bg-accent hover:underline"
                          >
                            {opLabel(op)}
                          </button>
                        ) : (
                          <span className="block truncate px-1 text-muted-foreground">{opLabel(op)}</span>
                        )}
                      </li>
                    );
                  })}
                </ul>
              ) : null}
              {/* The server rewinds baseline history only, so a branch changeset has no as-of view. */}
              {expanded === c.id && onViewAsOf && !c.branch ? (
                <div className="px-2 pb-2">
                  <Button variant="outline" size="sm" onClick={() => onViewAsOf(c.id)}>
                    View board as of this
                  </Button>
                </div>
              ) : null}
            </li>
          ))}
        </ul>
        {error ? <p className="text-[11px] italic text-muted-foreground">History unavailable ({error}).</p> : null}
        {loading ? <p className="text-[11px] text-muted-foreground">Loading…</p> : null}
        {nextPageToken && !loading ? (
          <Button variant="outline" size="sm" onClick={() => void load(nextPageToken)}>
            Load older
          </Button>
        ) : null}
      </div>
    </aside>
  );
}

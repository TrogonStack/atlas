import { useEffect, useRef, useState } from 'react';
import { JsonPane } from '@/components/ConflictInspector';
import { Button } from '@/components/ui/button';
import { api, type EntityRevision } from '@/lib/api';
import { readBranchFromUrl } from '@/lib/branch';
import type { Entity } from '@/lib/model';
import { cn } from '@/lib/utils';

// A put with no `before` created the key; naming it apart from an update
// lets a reader find where the entity began without opening every row.
function opLabel(r: EntityRevision): string {
  if (r.kind === 'KIND_DELETED') return 'deleted';
  return r.before ? 'updated' : 'created';
}

function formatAt(at: string): string {
  const d = new Date(at);
  return Number.isNaN(d.getTime()) ? at : d.toLocaleString();
}

// Mounted only while the History tab is open (base-ui unmounts inactive
// panels), so selecting an entity costs no history read until someone asks.
export function EntityHistory({ entity }: { entity: Entity }) {
  const [revisions, setRevisions] = useState<EntityRevision[]>([]);
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
      const page = await api.entityHistory({ kind: entity.kind, ...entity.id }, pageToken, {
        signal: ctrl.signal,
        branch: readBranchFromUrl(),
      });
      setRevisions((prev) => (pageToken ? [...prev, ...page.revisions] : page.revisions));
      setNextPageToken(page.nextPageToken);
    } catch (e) {
      if (ctrl.signal.aborted) return;
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (!ctrl.signal.aborted) setLoading(false);
    }
  };

  // The Inspector keys this panel by entity, so a new selection starts from
  // the newest page; unmounting cancels whatever page is in flight.
  useEffect(() => {
    void load();
    return () => abortRef.current?.abort();
    // oxlint-disable-next-line react-hooks/exhaustive-deps -- mount-only, see above
  }, []);

  return (
    <div className="space-y-2">
      {revisions.length === 0 && !loading && !error ? (
        <p className="text-[11px] italic text-muted-foreground">No recorded edits for this entity.</p>
      ) : null}
      <ul className="space-y-1">
        {revisions.map((r) => (
          <li key={r.changesetId} className="rounded-md border border-border">
            <button
              type="button"
              aria-expanded={expanded === r.changesetId}
              onClick={() => setExpanded((cur) => (cur === r.changesetId ? undefined : r.changesetId))}
              className={cn(
                'flex w-full flex-wrap items-center gap-x-2 px-2 py-1 text-left text-[10px] hover:bg-accent',
                expanded === r.changesetId && 'bg-accent',
              )}
            >
              <span className="font-semibold">{opLabel(r)}</span>
              <span className="text-muted-foreground">{formatAt(r.at)}</span>
              <span className="truncate">{r.author}</span>
              <span title={r.changesetId} className="ml-auto font-mono text-muted-foreground">
                {r.changesetId.slice(-12)}
              </span>
            </button>
            {expanded === r.changesetId ? (
              <div className="space-y-2 border-t border-border p-2">
                <JsonPane title="Before" value={r.before} />
                <JsonPane title="After" value={r.after} />
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
  );
}

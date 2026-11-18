// Top-bar tag for a board read as of a past changeset. Amber like BranchBar's
// branch preview, because both say "this is not the live baseline".
import { History } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api } from '@/lib/api';
import { withAsOfQuery } from '@/lib/branch';

function formatAt(at: string): string {
  const d = new Date(at);
  return Number.isNaN(d.getTime())
    ? at
    : d.toLocaleString(undefined, {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
        timeZoneName: 'short',
      });
}

export function AsOfTag({
  changesetId,
  incomplete,
  onExit,
}: {
  changesetId: string;
  incomplete?: boolean;
  onExit: () => void;
}) {
  const [at, setAt] = useState<string>();

  useEffect(() => {
    setAt(undefined);
    const ctrl = new AbortController();
    api
      .changeset(changesetId, { signal: ctrl.signal })
      .then((res) => {
        if (!ctrl.signal.aborted) setAt(res.changeset?.at || undefined);
      })
      // The id still names the point in time, so a failed lookup falls back to it.
      .catch(() => {});
    return () => ctrl.abort();
  }, [changesetId]);

  const when = at ? formatAt(at) : `changeset ${changesetId.slice(0, 8)}`;
  const { pathname, search, hash } = window.location;
  return (
    <div className="flex min-w-0 items-center gap-2">
      <output
        aria-label="As of"
        title={`The board right after changeset ${changesetId}. It does not update.`}
        className="flex h-8 min-w-0 items-center gap-1.5 rounded-md border border-amber-400 bg-amber-100 px-3 text-xs text-amber-900"
      >
        <History aria-hidden="true" className="h-4 w-4 shrink-0" />
        <span className="min-w-0 truncate">as of {when}</span>
        <span aria-hidden="true">·</span>
        <span className="shrink-0 rounded bg-amber-200 px-1 text-[9px] font-bold uppercase tracking-wide">
          read only
        </span>
        {incomplete ? (
          <span
            title="The server clipped this past board at its entity limit, so some members or events are missing."
            className="shrink-0 rounded bg-amber-300 px-1 text-[9px] font-bold uppercase tracking-wide"
          >
            partial
          </span>
        ) : null}
      </output>
      <a
        href={withAsOfQuery(`${pathname}${search}${hash}`, undefined)}
        onClick={(e) => {
          e.preventDefault();
          onExit();
        }}
        className="shrink-0 text-xs text-amber-900 underline hover:text-foreground"
      >
        Exit
      </a>
    </div>
  );
}

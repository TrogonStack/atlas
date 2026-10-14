// Branch indicator + switcher (Phase 3: Studio, requirements 3 & 4).
// Rendered in TopBar only when relevant. The trigger reads as state the way
// a git client's branch switcher does: it names the current ref ("baseline"
// or the branch) and turns amber with a read-only-preview tag while a branch
// is active. Opening it lists /api/branches plus "baseline" to exit, so a
// baseline user can jump onto a branch from anywhere in the studio.
//
// A hand-rolled disclosure rather than src/components/ui/dropdown-menu: its
// rows carry more than a menu item (doc, change count, async load and error
// states).
import { ChevronDown, GitBranch } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { api, type BranchInfo } from '@/lib/api';
import { cn } from '@/lib/utils';

function CurrentBadge() {
  return (
    <Badge variant="secondary" className="ml-auto px-1.5 text-[9px] font-semibold uppercase tracking-wide leading-3">
      current
    </Badge>
  );
}

export function BranchBar({
  branch,
  onSelectBranch,
  onOpenReview,
}: {
  branch?: string;
  onSelectBranch: (name: string | undefined) => void;
  /** When omitted, the Review affordance is hidden so it cannot be a dead click. */
  onOpenReview?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [branches, setBranches] = useState<BranchInfo[]>();
  const [error, setError] = useState<string>();
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    setBranches(undefined);
    setError(undefined);
    const ctrl = new AbortController();
    api
      .branches({ signal: ctrl.signal })
      .then((res) => {
        if (ctrl.signal.aborted) return;
        setBranches(res.branches ?? []);
      })
      .catch((e) => {
        if (ctrl.signal.aborted) return;
        setError(e instanceof Error ? e.message : String(e));
      });
    return () => ctrl.abort();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  }, [open]);

  return (
    <div ref={containerRef} className="relative flex min-w-0 items-center gap-2">
      {/* The name yields to the bar: it shrinks and ellipsizes with the available width, and caps so it cannot hog a wide bar either. */}
      <Button
        variant="outline"
        size="sm"
        aria-label={branch ? `Branch: ${branch}, read-only preview` : 'Branch: baseline'}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          'min-w-0',
          branch && 'border-amber-400 bg-amber-100 text-amber-900 hover:bg-amber-200 hover:text-amber-900',
        )}
      >
        <GitBranch />
        <span className={cn('min-w-0 max-w-[16rem] truncate', branch && 'font-mono')}>{branch ?? 'baseline'}</span>
        {branch ? (
          <span className="hidden shrink-0 rounded bg-amber-200 px-1 text-[9px] font-bold uppercase tracking-wide md:inline">
            read-only preview
          </span>
        ) : null}
        <ChevronDown className="opacity-60" />
      </Button>
      {branch && onOpenReview ? (
        <Button variant="outline" size="sm" onClick={onOpenReview}>
          Review
        </Button>
      ) : null}
      {open ? (
        <div className="absolute right-0 top-full z-20 mt-1 w-72 rounded-md border border-border bg-card p-1 shadow-md">
          <button
            type="button"
            onClick={() => {
              onSelectBranch(undefined);
              setOpen(false);
            }}
            className={cn(
              'flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-accent',
              !branch && 'font-semibold',
            )}
          >
            baseline
            {!branch ? <CurrentBadge /> : null}
          </button>
          {error ? <div className="px-2 py-1.5 text-[11px] text-red-600">{error}</div> : null}
          {branches === undefined && !error ? (
            <div className="px-2 py-1.5 text-[11px] text-muted-foreground">Loading branches…</div>
          ) : null}
          {branches?.length === 0 ? (
            <div className="px-2 py-1.5 text-[11px] text-muted-foreground">No branches yet.</div>
          ) : null}
          {branches?.map((b) => (
            <button
              key={b.name}
              type="button"
              onClick={() => {
                onSelectBranch(b.name);
                setOpen(false);
              }}
              className={cn(
                'flex w-full flex-col items-start gap-0.5 rounded px-2 py-1.5 text-left text-xs hover:bg-accent',
                branch === b.name && 'font-semibold',
              )}
            >
              <span className="flex w-full items-center gap-2">
                <span className="truncate font-mono">{b.name}</span>
                {branch === b.name ? <CurrentBadge /> : null}
              </span>
              {b.doc ? <span className="truncate text-[10px] font-normal text-muted-foreground">{b.doc}</span> : null}
              <span className="text-[9px] text-muted-foreground">{b.deltaCount} change(s)</span>
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

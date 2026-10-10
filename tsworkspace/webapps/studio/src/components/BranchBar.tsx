// Branch indicator + switcher (Phase 3: Studio, requirements 3 & 4).
// Rendered in TopBar only when relevant: the chip + read-only-preview hint
// always show when a branch is active; the picker (listing /api/branches
// plus "baseline" to exit) is available regardless, so a baseline user can
// jump onto a branch from anywhere in the studio.
//
// No headless-UI/popover library exists in this codebase (src/components/ui
// has only card/badge/separator/button/input); this is a hand-rolled
// disclosure, not a new modal abstraction.
import { GitBranch } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Button } from '@/components/ui/button';
import { api, type BranchInfo } from '@/lib/api';
import { cn } from '@/lib/utils';

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
    <div ref={containerRef} className="relative flex items-center gap-2">
      {branch ? (
        <div className="flex items-center gap-1.5 rounded-md border border-amber-400 bg-amber-100 px-2 py-1 text-xs font-medium text-amber-900">
          <GitBranch className="h-3.5 w-3.5" />
          <span className="max-w-[16rem] truncate font-mono">{branch}</span>
          <span className="rounded bg-amber-200 px-1 text-[9px] font-bold uppercase tracking-wide">
            read-only preview
          </span>
          {onOpenReview ? (
            <Button variant="ghost" size="sm" className="h-5 px-1.5 text-[10px]" onClick={onOpenReview}>
              Review
            </Button>
          ) : null}
        </div>
      ) : null}
      <Button variant="outline" size="sm" onClick={() => setOpen((v) => !v)}>
        <GitBranch />
        {branch ? 'Switch' : 'Branch'}
      </Button>
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
            {!branch ? <span className="ml-auto text-[9px] text-muted-foreground">current</span> : null}
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
                {branch === b.name ? <span className="ml-auto text-[9px] text-muted-foreground">current</span> : null}
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

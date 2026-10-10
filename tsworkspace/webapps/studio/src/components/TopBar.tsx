import { ChevronLeft, Library, RefreshCw, Workflow } from 'lucide-react';
import { BranchBar } from '@/components/BranchBar';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type { ServerInfo } from '@/lib/api';
import type { RealtimeStatus } from '@/lib/realtime';
import { cn } from '@/lib/utils';

export function TopBar({
  info,
  realtime,
  error,
  onRefresh,
  scopedTitle,
  backHref,
  branch,
  onSelectBranch,
  onOpenBranchReview,
}: {
  info?: ServerInfo;
  realtime: RealtimeStatus;
  error?: string;
  onRefresh: () => void;
  /** When set, shown after the studio name with the back chevron. */
  scopedTitle?: string;
  backHref?: string;
  /** Active branch, if any (Phase 3: Studio). Omit to hide the branch UI entirely. */
  branch?: string;
  onSelectBranch?: (name: string | undefined) => void;
  onOpenBranchReview?: () => void;
}) {
  return (
    <header className="flex h-12 items-center gap-3 border-b border-border bg-card px-4">
      {backHref ? (
        <a href={backHref} className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground">
          <ChevronLeft className="h-4 w-4" />
          Overview
        </a>
      ) : null}
      <Workflow className="h-5 w-5 text-primary" />
      <h1 className="text-sm font-semibold tracking-tight">Event Model Studio</h1>
      {scopedTitle ? (
        <>
          <span className="text-muted-foreground">/</span>
          <span className="truncate text-sm font-medium">{scopedTitle}</span>
        </>
      ) : null}
      {info?.schemaVersion ? <Badge variant="outline">{info.schemaVersion}</Badge> : null}
      <div className="ml-auto flex items-center gap-3">
        {error ? <span className="max-w-md truncate text-xs text-red-600">{error}</span> : null}
        <output
          aria-label="Live updates"
          title={
            realtime.detail ??
            {
              live: 'Live updates connected',
              connecting: 'Connecting to live updates',
              offline: 'Live updates disconnected. Use Refresh to get the latest model.',
            }[realtime.state]
          }
          className="flex items-center gap-1.5 text-xs text-muted-foreground"
        >
          <span
            aria-hidden="true"
            className={cn(
              'h-2 w-2 rounded-full',
              realtime.state === 'live'
                ? 'bg-emerald-500'
                : realtime.state === 'connecting'
                  ? 'bg-amber-400'
                  : 'bg-red-500',
            )}
          />
          {realtime.state}
          {realtime.detail ? <span className="sr-only">: {realtime.detail}</span> : null}
        </output>
        <Button variant="outline" size="sm" onClick={onRefresh}>
          <RefreshCw />
          Refresh
        </Button>
        <a
          href="/namespaces"
          className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border bg-card px-3 text-xs font-medium hover:bg-accent hover:text-accent-foreground"
        >
          <Library className="h-4 w-4" />
          Namespaces
        </a>
        {onSelectBranch ? (
          <BranchBar branch={branch} onSelectBranch={onSelectBranch} onOpenReview={onOpenBranchReview} />
        ) : null}
      </div>
    </header>
  );
}

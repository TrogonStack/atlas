import { Camera, ChevronLeft, History, Workflow } from 'lucide-react';
import { AsOfTag } from '@/components/AsOfTag';
import { BranchBar } from '@/components/BranchBar';
import { DownloadYamlButton } from '@/components/DownloadYamlButton';
import { TopBarMenu } from '@/components/TopBarMenu';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type { ServerInfo } from '@/lib/api';
import type { RealtimeStatus } from '@/lib/realtime';
import type { SnapshotStamp } from '@/lib/snapshot';
import { cn } from '@/lib/utils';

export function TopBar({
  info,
  realtime,
  error,
  onRefresh,
  scopedTitle,
  backHref,
  snapshot,
  onExportHtml,
  branch,
  onSelectBranch,
  onOpenBranchReview,
  onOpenHistory,
  asOf,
  asOfIncomplete,
  onExitAsOf,
}: {
  info?: ServerInfo;
  realtime: RealtimeStatus;
  error?: string;
  onRefresh: () => void;
  /** When set, shown after the studio name with the back chevron. */
  scopedTitle?: string;
  backHref?: string;
  /** Set when the page is a saved snapshot: live controls give way to when and where it was taken. */
  snapshot?: SnapshotStamp;
  /** Saves the board as one HTML file. Omit to hide the button. */
  onExportHtml?: () => Promise<void>;
  /** Active branch, if any (Phase 3: Studio). Omit to hide the branch UI entirely. */
  branch?: string;
  onSelectBranch?: (name: string | undefined) => void;
  onOpenBranchReview?: () => void;
  /** Opens the namespace history drawer. Omit to hide the button. */
  onOpenHistory?: () => void;
  /** Changeset the board is read as of: live, branch and menu controls give way to a read-only tag. */
  asOf?: string;
  asOfIncomplete?: boolean;
  onExitAsOf?: () => void;
}) {
  const historyButton = onOpenHistory ? (
    <Button variant="outline" size="sm" aria-label="Namespace history" onClick={onOpenHistory}>
      <History />
      <span className="hidden md:inline">History</span>
    </Button>
  ) : null;
  return (
    <header className="flex h-12 items-center gap-3 border-b border-border bg-card px-4">
      <div className="flex min-w-0 items-center gap-3">
        {backHref && !snapshot ? (
          <a
            href={backHref}
            className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
          >
            <ChevronLeft className="h-4 w-4" />
            Overview
          </a>
        ) : null}
        <Workflow className="h-5 w-5 shrink-0 text-primary" />
        <h1 className="sr-only text-sm font-semibold tracking-tight lg:not-sr-only lg:whitespace-nowrap">
          Event Model Studio
        </h1>
        {scopedTitle ? (
          <>
            <span className="hidden text-muted-foreground lg:inline">/</span>
            <span className="min-w-0 truncate text-sm font-medium">{scopedTitle}</span>
          </>
        ) : null}
        {info?.schemaVersion ? (
          <Badge variant="outline" className="shrink-0">
            {info.schemaVersion}
          </Badge>
        ) : null}
      </div>
      {/* min-w-0, not shrink-0: a long branch name must ellipsize here rather than push the title out of the bar. */}
      <div className="ml-auto flex min-w-0 items-center gap-3">
        {error ? <span className="max-w-md truncate text-xs text-red-600">{error}</span> : null}
        {snapshot ? (
          <>
            <SnapshotBadge snapshot={snapshot} serverVersion={info?.serverVersion} />
            <DownloadYamlButton snapshot={snapshot} />
          </>
        ) : asOf ? (
          <>
            {historyButton}
            <AsOfTag changesetId={asOf} incomplete={asOfIncomplete} onExit={() => onExitAsOf?.()} />
          </>
        ) : (
          <>
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
              className="flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground"
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
            {historyButton}
            {onSelectBranch ? (
              <BranchBar branch={branch} onSelectBranch={onSelectBranch} onOpenReview={onOpenBranchReview} />
            ) : null}
            <TopBarMenu onRefresh={onRefresh} onExportHtml={onExportHtml} />
          </>
        )}
      </div>
    </header>
  );
}

function SnapshotBadge({ snapshot, serverVersion }: { snapshot: SnapshotStamp; serverVersion?: string }) {
  const at = new Date(snapshot.capturedAt);
  const when = Number.isNaN(at.getTime())
    ? snapshot.capturedAt
    : at.toLocaleString(undefined, {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
        timeZoneName: 'short',
      });
  const parts = ['Snapshot', when, snapshot.branch ? `branch ${snapshot.branch}` : 'baseline'];
  if (serverVersion) parts.push(`server ${serverVersion}`);
  return (
    <output
      aria-label="Snapshot"
      title="A saved copy of this board. It does not update."
      className="flex items-center gap-1.5 text-xs text-muted-foreground"
    >
      <Camera aria-hidden="true" className="h-4 w-4" />
      {parts.join(' · ')}
    </output>
  );
}

import {
  AlertCircle,
  Check,
  Download,
  EllipsisVertical,
  Eye,
  LoaderCircle,
  type LucideIcon,
  RefreshCw,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { FeaturePreviewDialog } from '@/components/FeaturePreviewDialog';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { StudioFeatureFlag, StudioFeatureFlags } from '@/lib/feature-flags';
import { cn } from '@/lib/utils';

type ExportState = { kind: 'idle' | 'working' | 'done' } | { kind: 'failed'; message: string };

const TRIGGER: Record<ExportState['kind'], { Icon: LucideIcon; title: string; tone?: string; resetAfterMs?: number }> =
  {
    idle: { Icon: EllipsisVertical, title: 'More actions' },
    working: { Icon: LoaderCircle, title: 'Saving the board as HTML', tone: 'animate-spin' },
    done: { Icon: Check, title: 'Saved', tone: 'text-emerald-600', resetAfterMs: 1500 },
    failed: { Icon: AlertCircle, title: 'Export failed', tone: 'text-red-600', resetAfterMs: 4000 },
  };

export function TopBarMenu({
  onRefresh,
  onExportHtml,
  featureFlags,
  onFeatureFlagChange,
}: {
  onRefresh: () => void;
  /** Saves the board as one HTML file. Omit to hide the entry. */
  onExportHtml?: () => Promise<void>;
  featureFlags?: StudioFeatureFlags;
  onFeatureFlagChange?: (feature: StudioFeatureFlag, enabled: boolean) => void;
}) {
  const [exportState, setExportState] = useState<ExportState>({ kind: 'idle' });
  const [menuOpen, setMenuOpen] = useState(false);
  const [featurePreviewOpen, setFeaturePreviewOpen] = useState(false);
  const menuTrigger = useRef<HTMLButtonElement>(null);
  const { Icon, title, tone, resetAfterMs } = TRIGGER[exportState.kind];
  useEffect(() => {
    if (resetAfterMs === undefined) return;
    const timer = setTimeout(() => setExportState({ kind: 'idle' }), resetAfterMs);
    return () => clearTimeout(timer);
  }, [exportState, resetAfterMs]);

  return (
    <div className="flex items-center gap-2">
      <output className="sr-only">
        {exportState.kind === 'working' || exportState.kind === 'done' ? title : null}
      </output>
      {exportState.kind === 'failed' ? (
        <span role="alert" className="max-w-xs truncate text-xs text-red-600">
          <span className="sr-only">{title}: </span>
          {exportState.message}
        </span>
      ) : null}
      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <DropdownMenuTrigger
          ref={menuTrigger}
          render={<Button variant="outline" size="icon" aria-label="More actions" title={title} />}
        >
          <Icon className={cn(tone)} />
        </DropdownMenuTrigger>
        <DropdownMenuContent finalFocus={featurePreviewOpen ? false : menuTrigger}>
          <DropdownMenuItem onClick={onRefresh}>
            <RefreshCw />
            Refresh
          </DropdownMenuItem>
          {onExportHtml ? (
            <DropdownMenuItem
              disabled={exportState.kind === 'working'}
              title="Save this board as one HTML file that opens without Atlas"
              onClick={async () => {
                setExportState({ kind: 'working' });
                try {
                  await onExportHtml();
                  setExportState({ kind: 'done' });
                } catch (e) {
                  setExportState({ kind: 'failed', message: e instanceof Error ? e.message : String(e) });
                }
              }}
            >
              <Download />
              Export HTML
            </DropdownMenuItem>
          ) : null}
          {featureFlags && onFeatureFlagChange ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onClick={() => {
                  setMenuOpen(false);
                  setFeaturePreviewOpen(true);
                }}
              >
                <Eye />
                Feature Preview
              </DropdownMenuItem>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
      {featureFlags && onFeatureFlagChange ? (
        <FeaturePreviewDialog
          open={featurePreviewOpen}
          onOpenChange={setFeaturePreviewOpen}
          featureFlags={featureFlags}
          onFeatureFlagChange={onFeatureFlagChange}
          finalFocus={menuTrigger}
        />
      ) : null}
    </div>
  );
}

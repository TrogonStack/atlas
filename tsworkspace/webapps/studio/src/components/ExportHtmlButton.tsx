import { AlertCircle, Check, Download, LoaderCircle, type LucideIcon } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';

type ExportState = { kind: 'idle' | 'working' | 'done' } | { kind: 'failed'; message: string };

const VIEW: Record<ExportState['kind'], { Icon: LucideIcon; label: string; resetAfterMs?: number }> = {
  idle: { Icon: Download, label: 'Export HTML' },
  working: { Icon: LoaderCircle, label: 'Export HTML' },
  done: { Icon: Check, label: 'Saved', resetAfterMs: 1500 },
  failed: { Icon: AlertCircle, label: 'Export failed', resetAfterMs: 4000 },
};

export function ExportHtmlButton({ onExport }: { onExport: () => Promise<void> }) {
  const [state, setState] = useState<ExportState>({ kind: 'idle' });
  const { Icon, label, resetAfterMs } = VIEW[state.kind];
  useEffect(() => {
    if (resetAfterMs === undefined) return;
    const timer = setTimeout(() => setState({ kind: 'idle' }), resetAfterMs);
    return () => clearTimeout(timer);
  }, [state, resetAfterMs]);
  return (
    <Button
      variant="outline"
      size="sm"
      disabled={state.kind === 'working'}
      title={state.kind === 'failed' ? state.message : 'Save this board as one HTML file that opens without Atlas'}
      onClick={async () => {
        setState({ kind: 'working' });
        try {
          await onExport();
          setState({ kind: 'done' });
        } catch (e) {
          setState({ kind: 'failed', message: e instanceof Error ? e.message : String(e) });
        }
      }}
    >
      <Icon className={state.kind === 'working' ? 'animate-spin' : undefined} />
      {label}
    </Button>
  );
}

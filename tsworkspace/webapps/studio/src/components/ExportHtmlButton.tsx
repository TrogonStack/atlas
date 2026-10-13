import { AlertCircle, Check, Download, LoaderCircle } from 'lucide-react';
import { useState } from 'react';
import { Button } from '@/components/ui/button';

type ExportState = 'idle' | 'working' | 'done' | 'failed';

export function ExportHtmlButton({ onExport }: { onExport: () => Promise<void> }) {
  const [state, setState] = useState<ExportState>('idle');
  const [detail, setDetail] = useState<string>();
  return (
    <Button
      variant="outline"
      size="sm"
      disabled={state === 'working'}
      title={detail ?? 'Save this board as one HTML file that opens without Atlas'}
      onClick={async () => {
        setState('working');
        setDetail(undefined);
        try {
          await onExport();
          setState('done');
          setTimeout(() => setState('idle'), 1500);
        } catch (e) {
          setDetail(e instanceof Error ? e.message : String(e));
          setState('failed');
          setTimeout(() => setState('idle'), 4000);
        }
      }}
    >
      {state === 'working' ? (
        <LoaderCircle className="animate-spin" />
      ) : state === 'done' ? (
        <Check />
      ) : state === 'failed' ? (
        <AlertCircle />
      ) : (
        <Download />
      )}
      {state === 'done' ? 'Saved' : state === 'failed' ? 'Export failed' : 'Export HTML'}
    </Button>
  );
}

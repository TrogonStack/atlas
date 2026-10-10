import { AlertCircle, Check, Copy } from 'lucide-react';
import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

// navigator.clipboard requires a secure context; *.orb.local is plain http,
// so fall back to the hidden-textarea trick when it is unavailable.
async function copyText(text: string): Promise<boolean> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // fall through
    }
  }
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.style.position = 'fixed';
  ta.style.opacity = '0';
  document.body.appendChild(ta);
  ta.select();
  const ok = document.execCommand('copy');
  ta.remove();
  return ok;
}

type CopyState = 'idle' | 'copied' | 'failed';

export function CopyContextButton({
  getText,
  label = 'Copy context',
  copiedLabel = 'Copied',
  title,
  className,
}: {
  getText: () => string;
  label?: string;
  copiedLabel?: string;
  title?: string;
  className?: string;
}) {
  const [state, setCopyState] = useState<CopyState>('idle');
  return (
    <Button
      variant="outline"
      size="sm"
      className={cn('w-full', className)}
      title={title}
      onClick={async () => {
        const ok = await copyText(getText());
        if (ok) {
          setCopyState('copied');
          setTimeout(() => setCopyState('idle'), 1500);
        } else {
          setCopyState('failed');
          setTimeout(() => setCopyState('idle'), 2500);
        }
      }}
    >
      {state === 'copied' ? <Check /> : state === 'failed' ? <AlertCircle /> : <Copy />}
      {state === 'copied' ? copiedLabel : state === 'failed' ? 'Copy failed' : label}
    </Button>
  );
}

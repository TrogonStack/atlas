import type { Ref } from 'react';
import { SeverityIcon } from '@/components/SeverityIcon';
import type { IssueIndex, Severity } from '@/lib/issues';
import { cn } from '@/lib/utils';

const SEVERITY_PILL: Record<Severity, string> = {
  error: 'text-red-700',
  warning: 'text-amber-700',
  info: 'text-sky-700',
};

const ORDER: Severity[] = ['error', 'warning', 'info'];

export function ValidationSummary({
  issues,
  open,
  panelId,
  onToggle,
  triggerRef,
}: {
  issues: IssueIndex;
  open: boolean;
  panelId: string;
  onToggle: () => void;
  triggerRef?: Ref<HTMLButtonElement>;
}) {
  // A validator that did not run must not read as a clean model.
  if (issues.error)
    return (
      <div
        title={`Validation did not run: ${issues.error}. Nothing here says whether this model is valid.`}
        className="rounded-md border border-border bg-card px-2 py-1.5 text-xs italic text-muted-foreground shadow-sm"
      >
        validation unavailable
      </div>
    );

  if (issues.all.length === 0) return null;

  const present = ORDER.filter((s) => issues.counts[s] > 0);
  return (
    <button
      ref={triggerRef}
      type="button"
      onClick={onToggle}
      aria-label="Validation findings"
      aria-expanded={open}
      aria-controls={open ? panelId : undefined}
      title={`View ${issues.all.length} validation findings`}
      className="flex items-center gap-2 rounded-md border border-border bg-card px-2 py-1.5 text-xs shadow-sm hover:bg-accent"
    >
      {present.map((s) => (
        <span key={s} data-severity={s} className={cn('flex items-center gap-1 font-semibold', SEVERITY_PILL[s])}>
          <SeverityIcon severity={s} className="h-3 w-3" />
          {issues.counts[s]}
        </span>
      ))}
      {issues.unattached.length > 0 ? (
        <span className="font-mono text-[10px] text-muted-foreground">{issues.unattached.length} model-wide</span>
      ) : null}
    </button>
  );
}

import { ArrowRight, X } from 'lucide-react';
import { useState } from 'react';
import { CopyFixPromptButton } from '@/components/CopyFixPromptButton';
import { kindStyle } from '@/components/canvas/StickyNode';
import { Button } from '@/components/ui/button';
import type { Issue, IssueIndex, Severity } from '@/lib/issues';
import type { Model } from '@/lib/model';
import { cn } from '@/lib/utils';

const SEVERITY_COLOR: Record<Severity, string> = {
  error: 'text-red-700',
  warning: 'text-amber-700',
  info: 'text-sky-700',
};

const SEVERITY_LABEL: Record<Severity, string> = { error: 'Errors', warning: 'Warnings', info: 'Info' };
const FILTERS = ['all', 'error', 'warning', 'info'] as const;

export function ValidationInspector({
  id,
  model,
  issues,
  onClose,
  onSelect,
  canSelect,
}: {
  id: string;
  model: Model;
  issues: IssueIndex;
  onClose: () => void;
  onSelect?: (subject: NonNullable<Issue['subject']>) => void;
  canSelect?: (subject: NonNullable<Issue['subject']>) => boolean;
}) {
  const [severity, setSeverity] = useState<(typeof FILTERS)[number]>('all');
  const findings = severity === 'all' ? issues.all : issues.all.filter((issue) => issue.severity === severity);

  return (
    <aside
      id={id}
      aria-label="Validation findings"
      className="flex h-full min-h-0 w-80 max-w-[calc(100vw-3rem)] shrink-0 flex-col border-l border-border bg-card"
    >
      <div className="flex items-start justify-between gap-2 border-b border-border p-4">
        <div className="flex flex-col gap-1">
          <h2 className="text-base font-semibold leading-tight">Validation findings</h2>
          {!issues.error ? <p className="text-xs text-muted-foreground">{issues.all.length} findings</p> : null}
        </div>
        <Button variant="ghost" size="icon" aria-label="Close validation findings" onClick={onClose}>
          <X />
        </Button>
      </div>
      {!issues.error && issues.all.length > 0 ? (
        <fieldset className="flex flex-wrap gap-1 border-b border-border p-4" aria-label="Filter findings by severity">
          {FILTERS.map((value) => (
            <button
              key={value}
              type="button"
              aria-pressed={severity === value}
              onClick={() => setSeverity(value)}
              className={cn(
                'rounded px-2 py-1 text-xs hover:bg-accent',
                severity === value ? 'bg-accent font-semibold' : 'text-muted-foreground',
              )}
            >
              {value === 'all' ? `All (${issues.all.length})` : `${SEVERITY_LABEL[value]} (${issues.counts[value]})`}
            </button>
          ))}
        </fieldset>
      ) : null}
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto overscroll-contain p-4 text-sm">
        {issues.error ? (
          <output className="flex flex-col gap-2 text-muted-foreground">
            <span className="font-medium">Validation unavailable</span>
            <span className="break-words">{issues.error}</span>
            <span>Validation did not run. This does not establish whether the model is valid.</span>
          </output>
        ) : issues.all.length === 0 ? (
          <p className="text-muted-foreground">No validation findings.</p>
        ) : findings.length === 0 ? (
          <p className="text-muted-foreground">No findings at this severity.</p>
        ) : (
          findings.map((issue) => {
            const subject = issue.subject;
            const identity = subject ? `${subject.id.namespace}/${subject.id.slug}@${subject.id.version}` : undefined;
            const kind = subject?.kind.replace(/([A-Z])/g, ' $1').toLowerCase();
            const subjectStyle = subject ? kindStyle(subject.kind) : undefined;
            const SubjectIcon = subjectStyle?.icon;
            return (
              <article
                key={`${issue.severity}:${issue.code}:${subject?.kind}:${identity}:${issue.field}:${issue.message}`}
                className="flex shrink-0 flex-col gap-2 break-words rounded border border-border p-3"
              >
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className={cn('text-[10px] font-bold uppercase', SEVERITY_COLOR[issue.severity])}>
                    {issue.severity}
                  </span>
                  <span className="break-all font-mono text-[11px]">{issue.code}</span>
                </div>
                <p className="whitespace-pre-wrap text-xs leading-relaxed">{issue.message}</p>
                {subject && onSelect && (canSelect?.(subject) ?? true) ? (
                  <button
                    type="button"
                    aria-label={`Open ${kind} ${identity}`}
                    onClick={() => onSelect(subject)}
                    className="group flex w-full cursor-pointer items-center gap-2 rounded-md border border-transparent bg-muted/30 p-2 text-left transition-colors hover:border-border hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-card"
                  >
                    {subjectStyle && SubjectIcon ? (
                      <span
                        aria-hidden="true"
                        className={cn(
                          'flex h-7 w-7 shrink-0 items-center justify-center rounded border text-foreground/70',
                          subjectStyle.bg,
                          subjectStyle.border,
                        )}
                      >
                        <SubjectIcon className="h-3.5 w-3.5" />
                      </span>
                    ) : null}
                    <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <span className="text-[11px] font-medium text-foreground">{subjectStyle?.label ?? kind}</span>
                      <span className="break-words font-mono text-[11px] leading-relaxed text-muted-foreground">
                        {identity}
                      </span>
                    </span>
                    <ArrowRight
                      aria-hidden="true"
                      className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60 transition-colors group-hover:text-foreground/70"
                    />
                  </button>
                ) : (
                  <div className="flex flex-col gap-0.5 text-muted-foreground">
                    <span className="text-[11px] capitalize">{subject ? kind : 'Model-wide finding'}</span>
                    {identity ? <span className="break-all font-mono text-[11px]">{identity}</span> : null}
                  </div>
                )}
                {issue.field ? (
                  <div className="flex flex-wrap items-baseline gap-1 text-[11px] text-muted-foreground">
                    <span>Field:</span>
                    <span className="break-all font-mono">{issue.field}</span>
                  </div>
                ) : null}
                <CopyFixPromptButton model={model} issue={issue} />
              </article>
            );
          })
        )}
      </div>
    </aside>
  );
}

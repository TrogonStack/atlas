// One badge summarizing every validator finding on an entity.
//
// Deliberately generic: it knows severities, not rule codes. The board
// already carries hand-written badges for two specific rules
// (COMMAND_NO_EMITTED_EVENTS, COMMAND_MULTIPLE_ISSUERS, see layout.ts), each
// re-derived client-side from the model. Those can drift from the validator
// and have to be extended by hand for every new rule. This one covers every
// rule the server has, including ones written after this code.

import { SeverityIcon } from '@/components/SeverityIcon';
import type { Issue, Severity } from '@/lib/issues';
import { cn } from '@/lib/utils';

const SEVERITY_BADGE: Record<Severity, string> = {
  error: 'border-red-400 bg-red-100 text-red-800',
  warning: 'border-amber-400 bg-amber-100 text-amber-800',
  info: 'border-sky-300 bg-sky-100 text-sky-800',
};

/** Every finding, worst first, one per line. This is the whole explanation a card can carry. */
export function issuesTitle(issues: Issue[]): string {
  return issues.map((i) => `${i.severity.toUpperCase()} ${i.code}: ${i.message}`).join('\n\n');
}

export function IssueBadge({ issues, className }: { issues: Issue[]; className?: string }) {
  if (issues.length === 0) return null;
  // `for()` returns worst-first, so the head sets the badge's severity.
  const severity = issues[0].severity;
  return (
    <span
      title={issuesTitle(issues)}
      // The icon carries no text, so the severity has to stay queryable.
      data-severity={severity}
      className={cn(
        'inline-flex shrink-0 items-center gap-0.5 rounded border px-1 text-[8px] font-bold uppercase',
        SEVERITY_BADGE[severity],
        className,
      )}
    >
      <SeverityIcon severity={severity} className="h-2.5 w-2.5" />
      {issues.length}
    </span>
  );
}

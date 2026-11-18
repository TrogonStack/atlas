import { CopyFixPromptButton } from '@/components/CopyFixPromptButton';
import type { Issue, Severity } from '@/lib/issues';
import type { Model } from '@/lib/model';

const SEVERITY_STYLE: Record<Severity, string> = {
  error: 'border-red-300 bg-red-50 text-red-800',
  warning: 'border-amber-300 bg-amber-50 text-amber-900',
  info: 'border-sky-200 bg-sky-50 text-sky-800',
};

// The validator's findings for one entity, kept in their own section instead
// of inline with everything else a panel claims about the model. A silent
// panel and a clean panel used to look identical, so an entity the server
// rejects read as healthy. `error` is not swallowed: "validation did not
// run" and "validation found nothing" are different states and must not
// render the same.
export function IssueList({
  issues,
  error,
  model,
  momentKey,
}: {
  issues: Issue[];
  error?: string;
  model: Model;
  momentKey?: string;
}) {
  if (error)
    return (
      <p className="text-[11px] italic text-muted-foreground">
        unavailable ({error}). This panel cannot say whether the entity is valid.
      </p>
    );
  if (issues.length === 0)
    return <p className="text-[11px] italic text-muted-foreground">No validation findings for this entity.</p>;
  return (
    <div className="flex flex-col gap-2">
      {issues.map((i) => (
        <div
          key={[i.code, i.field, i.message].join('\u0000')}
          className={`flex flex-col gap-1.5 rounded border px-2 py-1.5 ${SEVERITY_STYLE[i.severity]}`}
        >
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-[9px] font-bold uppercase tracking-wide">{i.severity}</span>
            <span className="font-mono text-[10px]">{i.code}</span>
            {i.field ? <span className="font-mono text-[10px] opacity-70">{i.field}</span> : null}
          </div>
          {i.ruleTitle ? <div className="text-[11px] font-semibold">{i.ruleTitle}</div> : null}
          <p className="text-[11px] leading-snug">{i.message}</p>
          {i.defaultSeverity && i.defaultSeverity !== i.severity ? (
            <p className="text-[10px] italic opacity-70">rule default: {i.defaultSeverity}</p>
          ) : null}
          <CopyFixPromptButton model={model} issue={i} momentKey={momentKey} />
        </div>
      ))}
    </div>
  );
}

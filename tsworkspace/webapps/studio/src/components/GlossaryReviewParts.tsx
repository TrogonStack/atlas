import { useCallback, useId, useMemo, useState } from 'react';
import { compareGlossaries, inspectGlossaryRefresh } from '@/lib/api';
import {
  exactGlossaryId,
  glossaryHref,
  glossaryIdLabel,
  glossaryStateLabel,
  type GlossaryDocumentRef,
  type GlossarySelection,
} from '@/lib/glossary';
import type { EntityId } from '@/lib/model';
import { useGlossaryRead } from '@/lib/useGlossaryRead';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';

export const GLOSSARY_SELECT_CLASS =
  'h-9 rounded-md border border-input bg-card px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring';

export function GlossaryFailure({ message, retry }: { message: string; retry: () => void }) {
  return (
    <div
      role="alert"
      className="flex flex-wrap items-center gap-3 rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-800"
    >
      <span>{message}</span>
      <Button variant="outline" size="sm" onClick={retry}>
        Retry
      </Button>
    </div>
  );
}
export function GlossaryTermLink({ id, branch }: { id: EntityId; branch?: string }) {
  return (
    <a href={glossaryHref(id, branch)} className="break-all font-mono text-xs underline underline-offset-2">
      {glossaryIdLabel(id)}
    </a>
  );
}

export function GlossaryRefreshInspection({
  selection,
  documents,
  generationRevision,
  branch,
}: {
  selection: GlossarySelection;
  documents: GlossaryDocumentRef[];
  generationRevision: string;
  branch?: string;
}) {
  const [revision, setRevision] = useState(generationRevision);
  const revisionId = useId();
  const [submitted, setSubmitted] = useState<string>();
  const load = useCallback(
    (signal: AbortSignal) =>
      inspectGlossaryRefresh({ selection, documents, generationRevision: submitted ?? '' }, { signal, branch }),
    [selection, documents, submitted, branch],
  );
  const refresh = useGlossaryRead(submitted ? JSON.stringify([selection, documents, submitted, branch]) : null, load);
  return (
    <details className="rounded-md border border-border">
      <summary className="cursor-pointer p-3 text-sm font-semibold">Context refresh inspection</summary>
      <div className="flex flex-col gap-3 border-t border-border p-3">
        <p className="text-xs text-muted-foreground">
          Compare this context with the generation revision below. This inspection does not regenerate or publish
          entries.
        </p>
        <label htmlFor={revisionId} className="flex flex-col gap-2 text-xs">
          Generation revision
          <Input
            id={revisionId}
            value={revision}
            onChange={(event) => {
              setRevision(event.target.value);
              setSubmitted(undefined);
            }}
          />
        </label>
        <Button
          className="self-start"
          variant="outline"
          size="sm"
          disabled={!revision.trim()}
          onClick={() => {
            if (submitted === revision) refresh.retry();
            else setSubmitted(revision);
          }}
        >
          Check freshness
        </Button>
        {refresh.state.status === 'loading' ? <output>Checking freshness…</output> : null}
        {refresh.state.status === 'failed' ? (
          <GlossaryFailure message={refresh.state.error} retry={refresh.retry} />
        ) : null}
        {refresh.state.status === 'ready' ? (
          <div className="flex flex-col gap-2 text-xs">
            {refresh.state.value.incomplete ? <output>Refresh inspection is incomplete.</output> : null}
            {refresh.state.value.observations?.map((observation) => {
              const id = exactGlossaryId(observation.entry?.term?.id);
              return (
                <div key={id ? glossaryIdLabel(id) : observation.entry?.title}>
                  <span className="font-mono">{id ? glossaryIdLabel(id) : 'Entry unavailable'}</span>:{' '}
                  {glossaryStateLabel(observation.entry?.state)}
                  {observation.generationChanged ? ' · Generation revision changed' : ''}
                </div>
              );
            })}
          </div>
        ) : null}
      </div>
    </details>
  );
}

export function GlossaryComparisonPanel({
  selection,
  namespaces,
  labels,
  branch,
}: {
  selection: GlossarySelection;
  namespaces: string[];
  labels: Record<string, string>;
  branch?: string;
}) {
  const [other, setOther] = useState('');
  const [audience, setAudience] = useState('');
  const audienceId = useId();
  const [submitted, setSubmitted] = useState<{ namespace: string; audience: string }>();
  const right = useMemo<GlossarySelection>(
    () => ({
      namespaces: submitted ? [submitted.namespace] : [],
      ...(submitted?.audience ? { audience: submitted.audience } : {}),
      includeUnreviewed: selection.includeUnreviewed,
    }),
    [submitted, selection.includeUnreviewed],
  );
  const load = useCallback(
    (signal: AbortSignal) => compareGlossaries({ left: selection, right }, { signal, branch }),
    [selection, right, branch],
  );
  const comparison = useGlossaryRead(submitted ? JSON.stringify([selection, right, branch]) : null, load);
  return (
    <section className="flex flex-col gap-3 rounded-md border border-border bg-card p-4">
      <h2 className="text-sm font-semibold">Compare contexts</h2>
      <div className="flex flex-wrap items-end gap-3">
        <label className="flex min-w-48 flex-1 flex-col gap-2 text-xs">
          Compare with context
          <select
            className={GLOSSARY_SELECT_CLASS}
            value={other}
            onChange={(event) => {
              setOther(event.target.value);
              setSubmitted(undefined);
            }}
          >
            <option value="">Choose a context</option>
            {namespaces.map((namespace) => (
              <option key={namespace} value={namespace}>
                {labels[namespace] ?? namespace}
              </option>
            ))}
          </select>
        </label>
        <label htmlFor={audienceId} className="flex min-w-48 flex-1 flex-col gap-2 text-xs">
          Comparison audience
          <Input
            id={audienceId}
            placeholder="Any audience"
            value={audience}
            onChange={(event) => {
              setAudience(event.target.value);
              setSubmitted(undefined);
            }}
          />
        </label>
        <Button
          variant="outline"
          size="sm"
          disabled={!other}
          onClick={() => setSubmitted({ namespace: other, audience: audience.trim() })}
        >
          Compare contexts
        </Button>
      </div>
      {comparison.state.status === 'loading' ? <output className="text-xs">Comparing vocabulary…</output> : null}
      {comparison.state.status === 'failed' ? (
        <GlossaryFailure message={comparison.state.error} retry={comparison.retry} />
      ) : null}
      {comparison.state.status === 'ready' ? (
        <div className="flex flex-col gap-3 text-xs">
          <p className="text-muted-foreground">Differences are candidates for review, not language rulings.</p>
          {comparison.state.value.incomplete ? <output>Comparison is incomplete.</output> : null}
          {comparison.state.value.differences?.length === 0 ? (
            <p>No differences were reported for this selection.</p>
          ) : null}
          {comparison.state.value.differences?.map((difference) => {
            const left = exactGlossaryId(difference.left?.id);
            const target = exactGlossaryId(difference.right?.id);
            return (
              <div key={JSON.stringify(difference)} className="flex flex-col gap-2 rounded-md border border-border p-3">
                <p className="font-semibold">{difference.title || 'Vocabulary difference'}</p>
                <p>{difference.explanation || 'No explanation supplied.'}</p>
                <div className="flex flex-wrap gap-3">
                  {left ? <GlossaryTermLink id={left} branch={branch} /> : null}
                  {target ? <GlossaryTermLink id={target} branch={branch} /> : null}
                </div>
              </div>
            );
          })}
        </div>
      ) : null}
    </section>
  );
}

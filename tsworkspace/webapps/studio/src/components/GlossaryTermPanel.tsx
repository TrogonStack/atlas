import { useCallback, useMemo, useState } from 'react';
import { CopyContextButton } from '@/components/CopyContextButton';
import { GlossaryFailure, GlossaryRefreshInspection, GlossaryTermLink } from '@/components/GlossaryReviewParts';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { inspectGlossarySources } from '@/lib/api';
import {
  exactGlossaryId,
  glossaryIdLabel,
  glossaryRecord,
  glossaryStateLabel,
  glossaryText,
  sameGlossaryId,
  type GlossarySelection,
  type GlossaryTermDetail,
} from '@/lib/glossary';
import type { EntityId } from '@/lib/model';
import { useGlossaryRead } from '@/lib/useGlossaryRead';

function valueText(value: unknown): string {
  return glossaryText(glossaryRecord(value).value);
}

function EvidenceSource({
  evidence,
  state,
  reason,
  branch,
}: {
  evidence: Record<string, unknown>;
  state?: string;
  reason?: string;
  branch?: string;
}) {
  const id = exactGlossaryId(glossaryRecord(evidence.document).id);
  const [opened, setOpened] = useState(false);
  const load = useCallback(
    (signal: AbortSignal) => {
      const sourceId = exactGlossaryId(glossaryRecord(evidence.document).id);
      return inspectGlossarySources({ documents: sourceId ? [{ id: sourceId }] : [] }, { signal, branch });
    },
    [evidence, branch],
  );
  const source = useGlossaryRead(opened && id ? JSON.stringify([id, branch]) : null, load);
  const current =
    source.state.status === 'ready'
      ? source.state.value.sources?.find((item) => sameGlossaryId(exactGlossaryId(item.document?.id), id))
      : undefined;
  return (
    <div className="flex flex-col gap-3 rounded-md border border-border p-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <span className="break-all font-mono text-xs">{id ? glossaryIdLabel(id) : 'Source identity unavailable'}</span>
        <Badge variant="outline">{state ? glossaryStateLabel(state) : 'Not checked'}</Badge>
      </div>
      {reason ? <p className="text-xs text-muted-foreground">{reason}</p> : null}
      <blockquote className="whitespace-pre-wrap border-l-2 border-lime-300 pl-3">
        {glossaryText(evidence.quote) || 'No supporting quotation recorded.'}
      </blockquote>
      <dl className="grid gap-2 text-xs sm:grid-cols-2">
        <div>
          <dt className="text-muted-foreground">Passage</dt>
          <dd>{glossaryText(evidence.locator) || 'Unrecorded'}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Source incarnation</dt>
          <dd className="break-all font-mono">{glossaryText(evidence.ownerUid) || 'Unavailable'}</dd>
        </div>
        <div className="sm:col-span-2">
          <dt className="text-muted-foreground">Pinned source digest</dt>
          <dd className="break-all font-mono">
            {glossaryText(glossaryRecord(evidence.contentDigest).sha256) || 'Unavailable'}
          </dd>
        </div>
      </dl>
      {id ? (
        <Button className="self-start" variant="outline" size="sm" onClick={() => setOpened(true)}>
          Inspect source
        </Button>
      ) : null}
      {source.state.status === 'loading' ? <output>Loading current source…</output> : null}
      {source.state.status === 'failed' ? <GlossaryFailure message={source.state.error} retry={source.retry} /> : null}
      {source.state.status === 'ready' ? (
        <div className="flex flex-col gap-2 rounded-md bg-muted/50 p-3">
          <p className="text-xs font-semibold">Current source inspection</p>
          {source.state.value.incomplete ? (
            <output className="text-xs">
              Source inspection is incomplete.
            </output>
          ) : null}
          <p className="text-xs">{glossaryStateLabel(current?.state)}</p>
          {current?.text ? (
            <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words text-xs">{current.text}</pre>
          ) : (
            <p className="text-xs">Current source text is unavailable.</p>
          )}
        </div>
      ) : null}
    </div>
  );
}

export function GlossaryTermPanel({
  detail,
  selected,
  selection,
  branch,
}: {
  detail: GlossaryTermDetail;
  selected: EntityId;
  selection: GlossarySelection;
  branch?: string;
}) {
  const term = glossaryRecord(detail.entity?.term);
  const id = exactGlossaryId(term.id);
  const glossary = glossaryRecord(term.glossary);
  const review = glossaryRecord(glossary.review);
  const policy = glossaryRecord(glossary.policy);
  const evidence = useMemo(() => {
    const items = glossaryRecord(glossaryRecord(detail.entity?.term).glossary).evidence;
    return Array.isArray(items) ? items.map(glossaryRecord) : [];
  }, [detail.entity]);
  const documents = useMemo(
    () =>
      evidence
        .map((item) => exactGlossaryId(glossaryRecord(item.document).id))
        .filter((item): item is EntityId => item !== undefined)
        .map((item) => ({ id: item })),
    [evidence],
  );
  const [correction, setCorrection] = useState('');
  if (!sameGlossaryId(id, selected) || !sameGlossaryId(exactGlossaryId(detail.entry?.term?.id), selected))
    return <div role="alert">The response does not match the selected exact term.</div>;
  const preferred = exactGlossaryId(glossaryRecord(policy.preferredTerm).id);
  const usages = Array.isArray(term.embodiedBy) ? term.embodiedBy.map(glossaryRecord) : [];
  const usageLabels: Record<string, string> = {
    GLOSSARY_USAGE_ALLOWED: 'Allowed',
    GLOSSARY_USAGE_DEPRECATED: 'Deprecated',
    GLOSSARY_USAGE_PROHIBITED: 'Prohibited',
  };
  const usage = usageLabels[glossaryText(policy.usage)] ?? 'Undeclared';
  return (
    <section
      aria-label="Selected glossary entry"
      className="flex min-w-0 flex-col gap-5 rounded-lg border border-border bg-card p-5"
    >
      <div className="flex flex-col gap-2">
        <h2 className="text-xl font-semibold">{glossaryText(term.title) || selected.slug}</h2>
        <code className="break-all text-xs text-muted-foreground">{glossaryIdLabel(selected)}</code>
        <div className="flex flex-wrap gap-2">
          <Badge variant="secondary">{glossaryStateLabel(detail.entry?.state)}</Badge>
          <Badge variant="outline">
            {detail.entry?.published === true ? 'Published selection' : 'Not in published selection'}
          </Badge>
          <Badge variant="outline">{glossaryText(glossary.audience) || 'Audience undeclared'}</Badge>
        </div>
        {detail.entry?.reason ? <p className="text-xs text-muted-foreground">{detail.entry.reason}</p> : null}
        {detail.incomplete ? (
          <output className="text-xs">
            Entry inspection is incomplete. Some dependencies may be unavailable.
          </output>
        ) : null}
      </div>
      <p className="whitespace-pre-wrap text-sm leading-relaxed">
        {glossaryText(term.doc) || 'No definition recorded.'}
      </p>
      <details className="rounded-md border border-border">
        <summary className="cursor-pointer p-3 text-sm font-semibold">Recorded review</summary>
        <dl className="grid gap-3 border-t border-border p-3 text-xs sm:grid-cols-2">
          <div>
            <dt className="text-muted-foreground">Reviewer</dt>
            <dd>{glossaryText(review.actor) || 'Unrecorded'}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Recorded at</dt>
            <dd>{glossaryText(review.reviewedAt) || 'Unrecorded'}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Next review</dt>
            <dd>{valueText(glossary.nextReviewOn) || 'Undeclared'}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Generation revision</dt>
            <dd>{glossaryText(glossary.generationRevision) || 'Undeclared'}</dd>
          </div>
          <div className="sm:col-span-2">
            <dt className="text-muted-foreground">Review reason</dt>
            <dd>{glossaryText(review.reason) || 'Unrecorded'}</dd>
          </div>
        </dl>
      </details>
      <section className="flex flex-col gap-3">
        <h3 className="text-sm font-semibold">Source evidence</h3>
        {evidence.length === 0 ? (
          <p className="text-xs text-muted-foreground">No evidence recorded.</p>
        ) : (
          evidence.map((item, index) => {
            const matches = detail.evidence?.filter((resolution) => (resolution.evidenceIndex ?? 0) === index);
            const resolution = matches?.length === 1 ? matches[0] : undefined;
            return (
              <EvidenceSource
                key={JSON.stringify(item)}
                evidence={item}
                state={resolution?.state}
                reason={resolution?.reason}
                branch={branch}
              />
            );
          })
        )}
      </section>
      <section className="flex flex-col gap-2">
        <h3 className="text-sm font-semibold">Modeled usage</h3>
        {usages.length === 0 ? (
          <p className="text-xs text-muted-foreground">No modeled usage declared.</p>
        ) : (
          <ul className="flex flex-col gap-2 text-xs">
            {usages.map((reference) => {
              const target = exactGlossaryId(reference.id);
              return (
                <li key={JSON.stringify(reference)} className="flex flex-wrap gap-2">
                  <span className="text-muted-foreground">
                    {glossaryText(reference.kind).replace('ENTITY_KIND_', '').toLowerCase().replaceAll('_', ' ')}
                  </span>
                  <code>{target ? glossaryIdLabel(target) : 'Exact reference unavailable'}</code>
                </li>
              );
            })}
          </ul>
        )}
      </section>
      <section className="flex flex-col gap-2">
        <h3 className="text-sm font-semibold">Declared terminology policy</h3>
        <p className="text-sm">{usage}</p>
        {glossaryText(policy.rationale) ? (
          <p className="text-xs text-muted-foreground">{glossaryText(policy.rationale)}</p>
        ) : null}
        {preferred ? (
          <div className="flex flex-col gap-1 text-xs">
            <span>Preferred term</span>
            <GlossaryTermLink id={preferred} branch={branch} />
          </div>
        ) : null}
        {policy.effectiveFrom || policy.effectiveUntil ? (
          <p className="text-xs text-muted-foreground">
            Effective from {valueText(policy.effectiveFrom) || 'unbounded'} until{' '}
            {valueText(policy.effectiveUntil) || 'unbounded'} (end excluded).
          </p>
        ) : null}
      </section>
      <GlossaryRefreshInspection
        selection={selection}
        documents={documents}
        generationRevision={glossaryText(glossary.generationRevision)}
        branch={branch}
      />
      <section className="flex flex-col gap-3 border-t border-border pt-4">
        <h3 className="text-sm font-semibold">Request a correction</h3>
        <p className="text-xs text-muted-foreground">
          Describe the intended correction, then copy the context for an agent. This does not change the model or record
          approval.
        </p>
        <label className="flex flex-col gap-2 text-xs">
          Correction request
          <textarea
            className="min-h-24 rounded-md border border-input bg-background p-3 text-sm"
            value={correction}
            onChange={(event) => setCorrection(event.target.value)}
          />
        </label>
        <CopyContextButton
          label="Copy correction context"
          getText={() =>
            [
              `Review glossary term ${glossaryIdLabel(selected)}.`,
              `Current etag: ${detail.etag ?? detail.entry?.etag ?? 'unavailable'}`,
              `Backend state: ${glossaryStateLabel(detail.entry?.state)}`,
              `Published selection: ${detail.entry?.published === true}`,
              `Branch: ${branch ?? 'baseline'}`,
              `Definition: ${glossaryText(term.doc)}`,
              `Requested correction: ${correction.trim() || 'Inspect this entry and clarify the intended correction.'}`,
              `Evidence: ${JSON.stringify(evidence)}`,
              'Fetch the exact entry and its current dependencies before proposing a guarded CLI/MCP change. Preserve expert edits. This correction request is not approval.',
            ].join('\n')
          }
        />
      </section>
    </section>
  );
}

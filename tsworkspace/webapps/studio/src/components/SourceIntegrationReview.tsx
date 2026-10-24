import { useEffect, useState } from 'react';
import { inspectSource, type SourceInspection, type SourceQuery } from '@/lib/api';
import { readBranchFromUrl } from '@/lib/branch';

function Exact({ value }: { value: unknown }) {
  return <pre className="whitespace-pre-wrap break-words text-xs">{JSON.stringify(value, null, 2)}</pre>;
}
function SourceResult({
  query,
  request,
  branch,
}: {
  query: SourceQuery;
  request: Record<string, unknown>;
  branch?: string;
}) {
  const [context] = useState({ query, request, branch });
  const [page, setPage] = useState<string>();
  const [result, setResult] = useState<{ data?: SourceInspection; error?: string }>();
  useEffect(() => {
    const controller = new AbortController();
    setResult(undefined);
    inspectSource(
      context.query,
      { ...context.request, ...(page ? { pageToken: page } : {}) },
      { branch: context.branch, signal: controller.signal },
    ).then(
      (data) => {
        if (!controller.signal.aborted) setResult({ data });
      },
      (error: unknown) => {
        if (!controller.signal.aborted) setResult({ error: error instanceof Error ? error.message : String(error) });
      },
    );
    return () => controller.abort();
  }, [context, page]);
  const data = result?.data;
  const run = data?.run;
  const snapshot = data?.snapshot;
  const fidelity = data?.fidelity ?? run?.fidelity;
  return (
    <section className="flex flex-col gap-2" aria-live="polite">
      {!result && <p>Checking source backend context…</p>}
      {result?.error && (
        <p role="alert">Source inspection unavailable: {result.error}. Outcome and resolution remain unknown.</p>
      )}
      {data && (
        <>
          <p>
            Branch: {context.branch ?? 'baseline'}. Scope, revisions and evidence below are supplied by the backend.
          </p>
          {snapshot && (
            <p>
              Supplier declared coverage: {String(snapshot.declaredCoverage ?? 'UNSPECIFIED')}. This declaration does
              not verify completeness or authorize deletion.
            </p>
          )}
          {run && (
            <p>
              Run status: {String(run.status ?? 'UNKNOWN')}. Cancellation: {String(run.cancellation ?? 'UNSPECIFIED')}.
            </p>
          )}
          {(run?.recoveryRequired || run?.status === 'SOURCE_RUN_STATUS_OUTCOME_UNKNOWN') && (
            <output>
              Recovery required. Native publication outcome remains unsettled; do not infer rollback or retry under a
              new identity.
            </output>
          )}
          {(data.incomplete || data.overlay?.incomplete || run?.incomplete || data.nextPageToken) && (
            <p>
              Incomplete inspection or remaining pages. Missing records do not prove absence, complete safety or an
              empty loss report.
            </p>
          )}
          {run?.detailsOmitted === true && (
            <p>
              Run summary omits details. Inspect the result and operation pages for the exact reviewed plan and guards.
            </p>
          )}
          {fidelity ? (
            <>
              <h5 className="font-semibold">Backend fidelity and export checks</h5>
              <Exact value={fidelity} />
            </>
          ) : (
            <p>
              Fidelity/export checks are unavailable in this response. Lossless recovery and native reapply remain
              unproven.
            </p>
          )}
          <Exact value={data} />
          {data.nextPageToken && (
            <button type="button" onClick={() => setPage(data.nextPageToken)}>
              Read next page
            </button>
          )}
        </>
      )}
    </section>
  );
}

export function SourceIntegrationReview({ namespace, asOf }: { namespace: string; asOf?: string }) {
  const branch = readBranchFromUrl();
  const [registrationId, setRegistrationId] = useState('');
  const [snapshotId, setSnapshotId] = useState('');
  const [runId, setRunId] = useState('');
  const [query, setQuery] = useState<SourceQuery>('registrations');
  const [inspection, setInspection] = useState<{
    query: SourceQuery;
    request: Record<string, unknown>;
    key: string;
    submission: number;
  }>();
  const context = JSON.stringify({ namespace, branch, registrationId, snapshotId, runId, query });
  const historical = !!asOf;
  const needsRegistration = query !== 'registrations';
  const needsSnapshot = query === 'snapshot' || query === 'overlay';
  const needsRun = query === 'run' || query === 'run-results' || query === 'run-operations';
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">Source integration review</h3>
      <p>
        Source evidence retains declarations and exact pointers. It establishes no behavioral, ownership or
        authorization authority. Agents submit changes through CLI/MCP.
      </p>
      {historical ? (
        <p>Historical source hydration is unavailable. Current resolution is not evidence for this revision.</p>
      ) : (
        <>
          <label className="flex flex-col gap-1">
            Review records
            <select value={query} onChange={(event) => setQuery(event.target.value as SourceQuery)}>
              {(
                [
                  'registrations',
                  'registration',
                  'snapshots',
                  'snapshot',
                  'mappings',
                  'overlay',
                  'runs',
                  'run',
                  'run-results',
                  'run-operations',
                ] as SourceQuery[]
              ).map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
          </label>
          {needsRegistration && (
            <label className="flex flex-col gap-1">
              Exact registration ID
              <input value={registrationId} onChange={(event) => setRegistrationId(event.target.value)} />
            </label>
          )}
          {needsSnapshot && (
            <label className="flex flex-col gap-1">
              Exact snapshot ID
              <input value={snapshotId} onChange={(event) => setSnapshotId(event.target.value)} />
            </label>
          )}
          {needsRun && (
            <label className="flex flex-col gap-1">
              Exact run ID
              <input value={runId} onChange={(event) => setRunId(event.target.value)} />
            </label>
          )}
          <button
            type="button"
            disabled={(needsRegistration && !registrationId) || (needsSnapshot && !snapshotId) || (needsRun && !runId)}
            onClick={() => {
              const paged = query.endsWith('s') || query === 'overlay';
              const request: Record<string, unknown> = {
                ...(needsRegistration ? { registrationId } : { administrationNamespace: namespace }),
                ...(needsSnapshot ? { snapshotId } : {}),
                ...(needsRun ? { runId } : {}),
                ...(query === 'snapshot' ? { includeBytes: false } : {}),
                ...(paged ? { pageSize: 100 } : {}),
              };
              setInspection((current) => ({
                query,
                request,
                key: context,
                submission: (current?.submission ?? 0) + 1,
              }));
            }}
          >
            Inspect backend records
          </button>
          {inspection?.key === context && (
            <SourceResult
              key={`${context}:${inspection.submission}`}
              query={inspection.query}
              request={inspection.request}
              branch={branch}
            />
          )}
        </>
      )}
    </section>
  );
}

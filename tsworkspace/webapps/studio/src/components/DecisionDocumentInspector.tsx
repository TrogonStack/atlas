import { useEffect, useState } from 'react';
import {
  getDecisionDetail,
  getDocumentDetail,
  listIncomingDecisionDeclarations,
  listIncomingDocumentDeclarations,
  listSubjectDecisions,
  listSubjectDocuments,
} from '@/lib/api';
import { readAsOfFromUrl, readBranchFromUrl } from '@/lib/branch';
import type { CatalogInspection, CatalogSubject, DocumentSelection } from '@/lib/decision-document';
import type { Entity } from '@/lib/model';
import { responsibilitySubjects } from '@/lib/responsibility';

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}
function Exact({ value }: { value: unknown }) {
  return (
    <pre className="whitespace-pre-wrap break-words text-xs">{JSON.stringify(value, null, 2) ?? 'Undeclared'}</pre>
  );
}
function Query({ title, load }: { title: string; load: (signal: AbortSignal) => Promise<CatalogInspection> }) {
  // The keyed context owns this query's lifetime, including its initial loader.
  const [loadContext] = useState(() => load);
  const [result, setResult] = useState<{ data?: CatalogInspection; error?: string }>();
  useEffect(() => {
    const controller = new AbortController();
    loadContext(controller.signal).then(
      (data) => {
        if (!controller.signal.aborted) setResult({ data });
      },
      (error: unknown) => {
        if (!controller.signal.aborted) setResult({ error: error instanceof Error ? error.message : String(error) });
      },
    );
    return () => controller.abort();
  }, [loadContext]);
  return (
    <section className="flex flex-col gap-2">
      <h4 className="font-semibold">{title}</h4>
      {!result && <p>Checking backend context…</p>}
      {result?.error && <p role="alert">Catalog inspection unavailable: {result.error}. Resolution remains unknown.</p>}
      {result?.data && (
        <>
          <Exact value={result.data.scope} />
          {!result.data.scope && <p>Checked scope unavailable; complete absence remains unproven.</p>}
          {result.data.incomplete && <p>Incomplete inspection; missing records do not prove absence.</p>}
          {!!result.data.failures?.length && (
            <>
              <p>Backend resolution failures</p>
              <Exact value={result.data.failures} />
            </>
          )}
          <Exact value={result.data} />
          {!result.data.declarations?.length && (
            <p>No declarations returned. Interpret this only within the reported checked scope and completeness.</p>
          )}
        </>
      )}
    </section>
  );
}
function Authored({ entity }: { entity: Entity }) {
  const retained = record(entity.raw.retained);
  const link = record(entity.raw.link).value;
  return (
    <section className="flex flex-col gap-2">
      <h4 className="font-semibold">Loaded authored declarations</h4>
      <Exact value={{ id: entity.id, uid: entity.uid, ...entity.raw }} />
      {typeof retained.content === 'string' && (
        <>
          <p>
            Retained text, displayed inertly. Markdown links and markup are not activated. Declared anchors do not
            verify rendered headings.
          </p>
          <pre className="whitespace-pre-wrap break-words">{retained.content}</pre>
        </>
      )}
      {typeof link === 'string' && (
        <>
          <p>External link: unverified content and reachability.</p>
          <pre className="whitespace-pre-wrap break-words">{link}</pre>
        </>
      )}
    </section>
  );
}
function selections(entity: Entity): DocumentSelection[] {
  const document = { id: entity.id };
  const values: DocumentSelection[] = [{ document }];
  const digest = record(record(entity.raw.retained).digest).sha256;
  if (entity.uid && Array.isArray(entity.raw.anchors))
    for (const raw of entity.raw.anchors) {
      const anchorId = record(record(raw).anchorId).value;
      if (typeof anchorId !== 'string' || !anchorId) continue;
      const anchor = { document, ownerUid: entity.uid, anchorId: { value: anchorId } };
      values.push({ anchor });
      if (typeof digest === 'string') values.push({ anchor: { ...anchor, expectedDigest: { sha256: digest } } });
    }
  return values;
}
export function DecisionDocumentInspector({ entity, asOf: historicalAsOf }: { entity: Entity; asOf?: string }) {
  const asOf = historicalAsOf ?? readAsOfFromUrl();
  const branch = asOf ? undefined : readBranchFromUrl();
  const catalog = entity.kind === 'decision' || entity.kind === 'document';
  const subjects = catalog
    ? Array.isArray(entity.raw.subjects)
      ? entity.raw.subjects.flatMap((value) =>
          record(value).subject
            ? [{ label: 'Authored exact subject', subject: record(value).subject as CatalogSubject }]
            : [],
        )
      : []
    : responsibilitySubjects(entity);
  const [selected, setSelected] = useState<string>();
  const [documentChoice, setDocumentChoice] = useState<string>();
  const subject = subjects.find((item) => JSON.stringify(item.subject) === selected)?.subject ?? subjects[0]?.subject;
  const documents = selections(entity);
  const document = documents.find((item) => JSON.stringify(item) === documentChoice) ?? documents[0];
  const contextKey = JSON.stringify({
    key: entity.key,
    raw: entity.raw,
    uid: entity.uid,
    branch,
    asOf,
    subject,
    document,
  });
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">Decision and Document inspection</h3>
      <p>
        Catalog decisions record assertions, not approval authority or implementation evidence. Document selections
        preserve exact identity and byte pins.
      </p>
      {catalog && <Authored entity={entity} />}
      {subject && (
        <label className="flex flex-col gap-1">
          Exact catalog subject
          <select value={JSON.stringify(subject)} onChange={(event) => setSelected(event.target.value)}>
            {subjects.map((item, index) => (
              <option key={JSON.stringify(item.subject)} value={JSON.stringify(item.subject)}>
                {item.label} {index + 1}
              </option>
            ))}
          </select>
          <Exact value={subject} />
        </label>
      )}
      {entity.kind === 'document' && (
        <label className="flex flex-col gap-1">
          Exact document selection
          <select value={JSON.stringify(document)} onChange={(event) => setDocumentChoice(event.target.value)}>
            {documents.map((item) => (
              <option key={JSON.stringify(item)} value={JSON.stringify(item)}>
                {JSON.stringify(item)}
              </option>
            ))}
          </select>
        </label>
      )}
      {asOf ? (
        <p>
          Retained historical declarations only. Historical catalog hydration is unavailable; current resolution is not
          historical evidence.
        </p>
      ) : (
        <div key={contextKey} className="flex flex-col gap-3">
          {entity.kind === 'decision' && (
            <>
              <Query
                title="Decision detail"
                load={(signal) => getDecisionDetail({ id: entity.id }, false, { branch, signal })}
              />
              <Query
                title="Incoming decision declarations"
                load={(signal) => listIncomingDecisionDeclarations({ id: entity.id }, { branch, signal })}
              />
            </>
          )}
          {entity.kind === 'document' && (
            <>
              <Query title="Document detail" load={(signal) => getDocumentDetail(document, { branch, signal })} />
              <Query
                title="Incoming document declarations"
                load={(signal) => listIncomingDocumentDeclarations(document, { branch, signal })}
              />
            </>
          )}
          {subject && (
            <>
              <Query
                title="Decisions for exact subject"
                load={(signal) => listSubjectDecisions(subject, undefined, { branch, signal })}
              />
              <Query
                title="Documents for exact subject"
                load={(signal) => listSubjectDocuments(subject, { branch, signal })}
              />
            </>
          )}
        </div>
      )}
    </section>
  );
}

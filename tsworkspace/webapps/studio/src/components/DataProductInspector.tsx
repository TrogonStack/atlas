import { useEffect, useState } from 'react';
import { inspectDataProduct, type DataProductInspection, type DataProductPortState } from '@/lib/api';
import { readAsOfFromUrl, readBranchFromUrl } from '@/lib/branch';
import type { Entity } from '@/lib/model';

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}
function list(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.map(record) : [];
}
function value(value: unknown): string {
  if (value === undefined || value === null) return 'Undeclared';
  return typeof value === 'object' ? String(record(value).value ?? 'Unknown') : String(value);
}
export function selectionLabel(state?: string): string {
  return state?.replace('DATA_PRODUCT_SELECTION_STATE_', '').replaceAll('_', ' ').toLowerCase() || 'unknown';
}
export function offeringLabel(inspection?: DataProductInspection): string {
  if (!inspection?.product) return 'Unknown: backend inspection unavailable';
  const label: Record<string, string> = {
    DATA_PRODUCT_OFFERING_STATE_DRAFT: 'Draft offering',
    DATA_PRODUCT_OFFERING_STATE_MAPPED: 'Confirmed mapped offering',
    DATA_PRODUCT_OFFERING_STATE_UNRESOLVED: 'Unresolved',
    DATA_PRODUCT_OFFERING_STATE_UNKNOWN: 'Unknown: backend classification',
  };
  return label[inspection.offering ?? ''] ?? 'Unknown: backend classification';
}
function Detail({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="break-words">{children}</dd>
    </div>
  );
}
function ExactContent({ value: content }: { value: unknown }) {
  return <pre className="whitespace-pre-wrap break-words text-xs">{JSON.stringify(content, null, 2)}</pre>;
}
function Port({ port, state, input }: { port: Record<string, unknown>; state?: DataProductPortState; input: boolean }) {
  const data = record(port.data);
  const lineage = record(port.derivedFrom);
  return (
    <section className="flex flex-col gap-3 rounded border p-3">
      <h4 className="font-semibold">
        {input ? 'Input' : 'Output'}: {value(port.name)}
      </h4>
      <dl className="flex flex-col gap-2">
        <Detail label="Port ID">{value(port.portId)}</Detail>
        {port.doc !== undefined && <Detail label="Documentation">{value(port.doc)}</Detail>}
        <Detail label="Data state">
          {state ? selectionLabel(state.data) : port.data ? 'Unchecked selection' : 'Undeclared'}
        </Detail>
        {port.data !== undefined && (
          <>
            <Detail label="Data association ID">{value(data.connectionId)}</Detail>
            <Detail
              label={
                input && data.event
                  ? 'Event input: local fact requires backend emission confirmation'
                  : 'ReadModel selection (integration messages use ReadModels)'
              }
            >
              <ExactContent value={data.event ?? data.readModel} />
            </Detail>
          </>
        )}
        {!input && (
          <>
            <Detail label="Output contract">
              {port.contract ? (
                <>
                  <span>
                    {selectionLabel(state?.contract)}; declaration is not compatibility proof. Opaque selectors remain
                    unverified.
                  </span>
                  <ExactContent value={port.contract} />
                </>
              ) : (
                'Undeclared'
              )}
            </Detail>
            <Detail label="Declared lineage">
              {port.derivedFrom === undefined ? (
                'Unknown (absent declaration)'
              ) : list(lineage.inputs).length ? (
                <ExactContent value={lineage} />
              ) : (
                'Explicit empty declaration; real dependencies remain unproven'
              )}
            </Detail>
            <Detail label="Backend lineage state">{selectionLabel(state?.lineage)}</Detail>
          </>
        )}
      </dl>
      {!input && (
        <>
          <h5 className="font-semibold">Explicit access</h5>
          {!list(port.access).length && <p>Undeclared</p>}
          {list(port.access).map((access) => {
            const checked = state?.access?.find(
              (item) => record(item.declaration?.connectionId).value === record(access.connectionId).value,
            );
            const operation = record(access.operation);
            return (
              <dl className="flex flex-col gap-2 rounded border p-2" key={value(access.connectionId)}>
                <Detail label="Access association ID">{value(access.connectionId)}</Detail>
                <Detail label="Selection">
                  {access.store
                    ? 'Store attachment'
                    : access.channel
                      ? 'Channel binding and observation'
                      : access.operation
                        ? 'API operation RESPONSE'
                        : 'Unknown selection'}
                </Detail>
                <Detail label="Backend access state">{selectionLabel(checked?.state)}</Detail>
                {access.operation !== undefined && (
                  <Detail label="Provider">
                    {operation.providerExposure === undefined
                      ? 'Undeclared (contract-level access)'
                      : selectionLabel(checked?.provider)}
                  </Detail>
                )}
                <ExactContent value={access} />
              </dl>
            );
          })}
        </>
      )}
      {state?.incomplete && <p>Incomplete backend inspection; no negative or verified conclusion follows.</p>}
    </section>
  );
}
export function DataProductInspector({ entity }: { entity: Entity }) {
  const branch = readBranchFromUrl();
  const asOf = readAsOfFromUrl();
  const [result, setResult] = useState<{ key: string; inspection?: DataProductInspection; error?: string }>();
  const key = `${entity.key}:${JSON.stringify(entity.raw)}:${branch ?? ''}:${asOf ?? ''}`;
  useEffect(() => {
    if (entity.kind !== 'dataProduct' || asOf) return;
    const controller = new AbortController();
    inspectDataProduct(entity.id, { branch, signal: controller.signal }).then(
      (inspection) => {
        if (!controller.signal.aborted) setResult({ key, inspection });
      },
      (error: unknown) => {
        if (!controller.signal.aborted)
          setResult({ key, error: error instanceof Error ? error.message : String(error) });
      },
    );
    return () => controller.abort();
  }, [entity.kind, entity.id, key, branch, asOf]);
  if (entity.kind !== 'dataProduct') return null;
  const current = result?.key === key ? result : undefined;
  const inspection = current?.inspection;
  const product = inspection?.product ?? entity.raw;
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">DataProduct inspection</h3>
      {current?.error && <p role="alert">Backend inspection unavailable: {current.error}</p>}
      {!inspection && !current?.error && (
        <p>{asOf ? 'Historical inspection unavailable; retained declarations only.' : 'Checking backend context…'}</p>
      )}
      <dl className="flex flex-col gap-2">
        <Detail label="Purpose">{value(product.purpose)}</Detail>
        <Detail label="Ownership">Undeclared pending responsibility layer</Detail>
        <Detail label="Checked context">
          {inspection?.scope ? <ExactContent value={{ ...inspection.scope, etag: inspection.etag }} /> : 'Unknown'}
        </Detail>
      </dl>
      {inspection?.incomplete && <p>Incomplete backend evidence. Missing paths do not prove no relationship.</p>}
      {(['inputs', 'outputs'] as const).map((direction) =>
        list(product[direction]).map((port) => (
          <Port
            key={`${direction}:${value(port.portId)}`}
            port={port}
            input={direction === 'inputs'}
            state={inspection?.ports?.find(
              (state) =>
                state.port?.portId?.value === record(port.portId).value &&
                state.port?.direction ===
                  (direction === 'inputs' ? 'DATA_PORT_DIRECTION_INPUT' : 'DATA_PORT_DIRECTION_OUTPUT'),
            )}
          />
        )),
      )}
      <p>{offeringLabel(inspection)}</p>
      <h4 className="font-semibold">Backend confirmed local Event emissions</h4>
      {inspection?.emissions?.length ? (
        <ExactContent value={inspection.emissions} />
      ) : (
        <p>No checked witnesses returned; this is not proof of absence.</p>
      )}
      <h4 className="font-semibold">Backend verified producing model evidence</h4>
      <p>Separate from declared lineage. Model paths are not runtime execution proof.</p>
      {inspection?.producing?.length ? (
        <ExactContent value={inspection.producing} />
      ) : (
        <p>Unknown: no checked producing path returned.</p>
      )}
    </section>
  );
}

import { useEffect, useRef, useState } from 'react';
import { inspectHumanInteraction } from '@/lib/api';
import { readAsOfFromUrl, readBranchFromUrl } from '@/lib/branch';
import {
  sliceOccurrences,
  type HumanInteractionRequest,
  type HumanInteractionResponse,
  type HumanPathType,
  type SliceOccurrence,
} from '@/lib/human-interaction';
import { hasUnambiguousOwnerIdentities } from '@/lib/responsibility';
import { Exact } from './DecisionDocumentInspector';
import { ExactReference } from './ArchitectureResourceDetails';
import { hostingConnection, uiHostings, type UiHostingView } from '@/lib/ui-hosting';
import type { Entity, Model } from '@/lib/model';

export function UiHostingDetails({
  entity,
  model,
  onSelect,
  asOf: historicalAsOf,
}: {
  entity: Entity;
  model: Model;
  onSelect: (key: string) => void;
  asOf?: string;
}) {
  if (entity.kind !== 'component') return null;
  const hostings = uiHostings(entity);
  const asOf = historicalAsOf ?? readAsOfFromUrl();
  const branch = asOf ? undefined : readBranchFromUrl();
  const context = JSON.stringify({
    branch,
    asOf,
    key: entity.key,
    uid: entity.uid,
    rows: model.entities.map((row) => [row.key, row.uid, row.raw]),
  });
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">UI hosting and human interaction</h3>
      <p>Authored UI hosting assertions do not establish deployment, permissions or runtime execution.</p>
      {!hostings.length && <p>No UI hosting assertions in this loaded owner.</p>}
      {hostings.map((item) => (
        <section
          className="flex flex-col gap-2 rounded border p-2"
          key={JSON.stringify([entity.uid, item.path, item.connectionId])}
        >
          <h4 className="font-semibold">{item.label ?? 'Unavailable label'}</h4>
          <dl className="flex flex-col gap-1">
            <dt>Owner UID</dt>
            <dd>{entity.uid || 'Unavailable'}</dd>
            <dt>Connection ID</dt>
            <dd>{item.connectionId || 'Unassigned'}</dd>
            <dt>Diagnostic path</dt>
            <dd>{item.path}</dd>
            <dt>Hosted UI</dt>
            <dd>
              {item.ui ? (
                <ExactReference kind="ui" reference={item.ui} model={model} onSelect={onSelect} />
              ) : (
                'Unavailable typed UI'
              )}
            </dd>
          </dl>
          {item.doc !== undefined && <p className="whitespace-pre-wrap">{item.doc}</p>}
        </section>
      ))}
      {!!hostings.length && (
        <>
          <p>
            Select an exact hosting and a complete slice tuple before checking an interaction. Loaded memberships do not
            establish a human path.
          </p>
          {asOf ? (
            <p>Retained historical hosting only. Current interaction inspection is unavailable for historical views.</p>
          ) : (
            <InteractionCheck key={context} hostings={hostings} model={model} branch={branch} />
          )}
        </>
      )}
    </section>
  );
}

function OccurrenceChoice({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: SliceOccurrence[];
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="flex flex-col gap-1">
      {label}
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        <option value="">Choose an exact occurrence</option>
        {options.map((item) => (
          <option key={item.connection.id.value} value={item.connection.id.value}>
            {item.path}: {item.connection.id.value}
          </option>
        ))}
      </select>
    </label>
  );
}

function InteractionCheck({ hostings, model, branch }: { hostings: UiHostingView[]; model: Model; branch?: string }) {
  const [hostingId, setHostingId] = useState('');
  const [pathType, setPathType] = useState<HumanPathType>('issuance');
  const [sliceKey, setSliceKey] = useState('');
  const [personaId, setPersonaId] = useState('');
  const [uiId, setUiId] = useState('');
  const [endpointId, setEndpointId] = useState('');
  const [result, setResult] = useState<HumanInteractionResponse>();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const active = useRef<AbortController | undefined>(undefined);
  useEffect(() => () => active.current?.abort(), []);
  const slices = model.entities.filter(
    (owner) =>
      owner.kind === (pathType === 'issuance' ? 'commandSlice' : 'uiSlice') &&
      !!owner.uid &&
      hasUnambiguousOwnerIdentities(owner),
  );
  const slice = slices.find((owner) => owner.key === sliceKey);
  const personas = slice ? sliceOccurrences(slice, 'persona') : [];
  const uis = slice ? sliceOccurrences(slice, 'ui') : [];
  const endpoints = slice ? sliceOccurrences(slice, pathType === 'issuance' ? 'command' : 'sourceReadModels') : [];
  const hosting = hostings.find((item) => item.connectionId === hostingId);
  const selectedHosting = hosting && hostingConnection(hosting);
  const persona = personas.find((item) => item.connection.id.value === personaId)?.connection;
  const ui = uis.find((item) => item.connection.id.value === uiId)?.connection;
  const endpoint = endpoints.find((item) => item.connection.id.value === endpointId)?.connection;
  const request: HumanInteractionRequest | undefined =
    selectedHosting && persona && ui && endpoint
      ? {
          hosting: selectedHosting,
          ...(pathType === 'issuance'
            ? { issuance: { persona, ui, command: endpoint } }
            : { display: { persona, ui, sourceReadModel: endpoint } }),
        }
      : undefined;
  const clear = () => {
    active.current?.abort();
    setResult(undefined);
    setError(undefined);
    setPending(false);
  };
  const check = async () => {
    if (!request) return;
    clear();
    const controller = new AbortController();
    active.current = controller;
    setPending(true);
    try {
      const response = await inspectHumanInteraction(request, { branch, signal: controller.signal });
      if (!controller.signal.aborted) setResult(response);
    } catch (failure: unknown) {
      if (!controller.signal.aborted) setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      if (!controller.signal.aborted) setPending(false);
    }
  };
  const choices = hostings.filter((item) => hostingConnection(item));
  const state =
    result?.state === 'CATALOG_RESOLUTION_STATE_RESOLVED'
      ? 'Resolved checked path'
      : result?.state === 'CATALOG_RESOLUTION_STATE_UNAVAILABLE'
        ? 'Unavailable endpoint'
        : result?.state === 'CATALOG_RESOLUTION_STATE_UNRESOLVED'
          ? 'Unresolved selection'
          : 'Resolution unknown';
  return (
    <section className="flex flex-col gap-3">
      <p>Choices below are limited to loaded owners. Only the backend check resolves the selected interaction.</p>
      <label className="flex flex-col gap-1">
        Exact hosting occurrence
        <select
          value={hostingId}
          onChange={(event) => {
            clear();
            setHostingId(event.target.value);
          }}
        >
          <option value="">Choose an exact hosting</option>
          {choices.map((item) => (
            <option key={item.connectionId} value={item.connectionId}>
              {item.path}: {item.connectionId}
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1">
        Human path type
        <select
          value={pathType}
          onChange={(event) => {
            clear();
            setPathType(event.target.value as HumanPathType);
            setSliceKey('');
            setPersonaId('');
            setUiId('');
            setEndpointId('');
          }}
        >
          <option value="issuance">Human issuance</option>
          <option value="display">Human display</option>
        </select>
      </label>
      <label className="flex flex-col gap-1">
        Exact slice owner
        <select
          value={sliceKey}
          onChange={(event) => {
            clear();
            setSliceKey(event.target.value);
            setPersonaId('');
            setUiId('');
            setEndpointId('');
          }}
        >
          <option value="">Choose an exact slice</option>
          {slices.map((owner) => (
            <option key={owner.key} value={owner.key}>
              {owner.title}: {owner.id.namespace}/{owner.id.slug}@{owner.id.version} ({owner.uid})
            </option>
          ))}
        </select>
      </label>
      <OccurrenceChoice
        label="Persona occurrence"
        options={personas}
        value={personaId}
        onChange={(value) => {
          clear();
          setPersonaId(value);
        }}
      />
      <OccurrenceChoice
        label="UI occurrence"
        options={uis}
        value={uiId}
        onChange={(value) => {
          clear();
          setUiId(value);
        }}
      />
      <OccurrenceChoice
        label={pathType === 'issuance' ? 'Command occurrence' : 'ReadModel observation occurrence'}
        options={endpoints}
        value={endpointId}
        onChange={(value) => {
          clear();
          setEndpointId(value);
        }}
      />
      <button type="button" disabled={!request || pending} onClick={() => void check()}>
        Check selected interaction
      </button>
      {pending && <p>Checking selected backend context…</p>}
      {error && <p role="alert">Interaction inspection unavailable: {error}. Resolution remains unknown.</p>}
      {result && (
        <section className="flex flex-col gap-2">
          <h4 className="font-semibold">{state}</h4>
          <p>Per-owner checked read set</p>
          <p>
            This receipt is not an atomic snapshot, runtime execution evidence or authorization for a write. It may be
            stale immediately; refresh the check after changes.
          </p>
          <Exact value={result.scope} />
          <Exact value={result.request} />
          <Exact value={result.edges} />
        </section>
      )}
    </section>
  );
}

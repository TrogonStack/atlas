import type { ReactNode } from 'react';
import { asId, type Entity, entityKey, type EntityKind, type Model, wireKind } from '@/lib/model';

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function list(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.map(record) : [];
}

function valueOf(value: unknown): string {
  if (value === undefined || value === null) return 'Undeclared';
  if (typeof value === 'object') return valueOf(record(value).value);
  return String(value);
}

function enumLabel(value: unknown, prefix: string): string {
  return valueOf(value).replace(prefix, '').replaceAll('_', ' ').toLowerCase();
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="break-words">{children}</dd>
    </div>
  );
}

export function ExactReference({
  kind,
  reference,
  model,
  onSelect,
}: {
  kind: EntityKind;
  reference: unknown;
  model: Model;
  onSelect: (key: string) => void;
}) {
  const id = asId(record(reference).id);
  const key = entityKey(kind, id);
  const target = model.entities.find((entity) => entity.key === key);
  const label = `${id.namespace}/${id.slug}@${id.version}`;
  return target ? (
    <button type="button" className="text-left underline" onClick={() => onSelect(key)}>
      {label}
    </button>
  ) : (
    <span>{label} (not loaded)</span>
  );
}

export function ArchitectureResourceDetails({
  entity,
  model,
  onSelect,
}: {
  entity: Entity;
  model: Model;
  onSelect: (key: string) => void;
}) {
  if (entity.kind !== 'channel' && entity.kind !== 'dataStore') return null;
  const raw = entity.raw;
  return (
    <section className="flex flex-col gap-3">
      <h3 className="text-xs font-semibold uppercase text-muted-foreground">
        {entity.kind === 'channel' ? 'Transport declaration' : 'Storage declaration'}
      </h3>
      {entity.kind === 'channel' ? (
        <>
          <dl className="flex flex-col gap-3">
            <Row label="Address template">{valueOf(record(raw.address).template)}</Row>
            <Row label="Protocols">
              {list(raw.protocols)
                .map((protocol) => valueOf(protocol))
                .join(', ') || 'Undeclared'}
            </Row>
            <Row label="Delivery guarantee">{enumLabel(raw.deliveryGuarantee, 'DELIVERY_GUARANTEE_')}</Row>
            {raw.component ? (
              <Row label="Broker component">
                <ExactReference kind="component" reference={raw.component} model={model} onSelect={onSelect} />
              </Row>
            ) : null}
            {raw.externalSystem ? (
              <Row label="Broker external system">
                <ExactReference
                  kind="externalSystem"
                  reference={raw.externalSystem}
                  model={model}
                  onSelect={onSelect}
                />
              </Row>
            ) : null}
          </dl>
          {list(record(raw.address).parameters).map((parameter) => (
            <dl className="flex flex-col gap-2 rounded border p-2" key={valueOf(parameter.name)}>
              <Row label="Address parameter">{valueOf(parameter.name)}</Row>
              {parameter.description !== undefined ? (
                <Row label="Description">{valueOf(parameter.description)}</Row>
              ) : null}
              {parameter.allowedValues !== undefined ? (
                <Row label="Allowed values">{JSON.stringify(record(parameter.allowedValues).values)}</Row>
              ) : null}
              {parameter.default !== undefined ? <Row label="Default">{valueOf(parameter.default)}</Row> : null}
              {Array.isArray(parameter.examples) && parameter.examples.length ? (
                <Row label="Examples">{JSON.stringify(parameter.examples)}</Row>
              ) : null}
            </dl>
          ))}
          <h4 className="font-semibold">Observation bindings</h4>
          {list(raw.bindings).map((binding) => {
            const observation = record(binding.observation);
            const owner = record(observation.owner);
            const kind = wireKind(owner.kind);
            const slice =
              kind === 'automationSlice' && typeof observation.ownerUid === 'string' && observation.ownerUid !== ''
                ? model.slices.find(
                    (slice) =>
                      slice.entity.key === entityKey(kind, asId(owner.id)) && slice.entity.uid === observation.ownerUid,
                  )
                : undefined;
            const source = record(observation.id).value
              ? slice?.readModels.find((edge) => edge.connectionId?.value === record(observation.id).value)
              : undefined;
            return (
              <dl className="flex flex-col gap-2 rounded border p-2" key={valueOf(binding.connectionId)}>
                <Row label="Binding ID">{valueOf(binding.connectionId)}</Row>
                <Row label="Observation owner">
                  {kind ? (
                    <ExactReference kind={kind} reference={owner} model={model} onSelect={onSelect} />
                  ) : (
                    'Unknown owner kind'
                  )}
                </Row>
                <Row label="Owner UID">{valueOf(observation.ownerUid)}</Row>
                <Row label="Observation connection ID">{valueOf(observation.id)}</Row>
                {source && slice?.processor ? (
                  <Row label="Observed ReadModel → Processor">{`${source.id.namespace}/${source.id.slug}@${source.id.version} → ${slice.processor.id.namespace}/${slice.processor.id.slug}@${slice.processor.id.version}`}</Row>
                ) : (
                  <Row label="Observation status">Not resolved in the loaded model</Row>
                )}
                {Object.keys(record(binding.parameterValues)).length ? (
                  <Row label="Parameter values">
                    {Object.entries(record(binding.parameterValues)).map(([name, value]) => (
                      <div key={name}>
                        {name}: {JSON.stringify(value)}
                      </div>
                    ))}
                  </Row>
                ) : null}
                {binding.doc ? <Row label="Binding documentation">{valueOf(binding.doc)}</Row> : null}
              </dl>
            );
          })}
        </>
      ) : (
        <>
          <dl className="flex flex-col gap-3">
            <Row label="Store class">
              {record(raw.storeClass).other !== undefined
                ? valueOf(record(raw.storeClass).other)
                : enumLabel(record(raw.storeClass).known, 'KNOWN_')}
            </Row>
            <Row label="Technology">{valueOf(raw.technology)}</Row>
            <Row label="Purpose">{valueOf(raw.purpose)}</Row>
            <Row label="Authoritative">
              {raw.authoritative === undefined ? 'Unknown' : raw.authoritative === true ? 'Yes' : 'No'}
            </Row>
            <Row label="Access mode">{enumLabel(raw.accessMode, 'ACCESS_MODE_')}</Row>
            <Row label="Classification">{enumLabel(raw.classification, 'CLASSIFICATION_')}</Row>
            {raw.residency ? (
              <>
                <Row label="Residency policy">{valueOf(record(raw.residency).policy)}</Row>
                {record(raw.residency).regions ? (
                  <Row label="Regions">
                    {list(record(record(raw.residency).regions).values)
                      .map(valueOf)
                      .join(', ')}
                  </Row>
                ) : null}
              </>
            ) : null}
            {raw.retention ? (
              <>
                <Row label="Retention policy">{valueOf(record(raw.retention).policy)}</Row>
                {record(raw.retention).duration ? (
                  <Row label="Retention duration">
                    {valueOf(record(record(raw.retention).duration).seconds)} seconds
                  </Row>
                ) : null}
              </>
            ) : null}
          </dl>
          <h4 className="font-semibold">ReadModel attachments</h4>
          {list(raw.attachments).map((attachment) => (
            <dl className="flex flex-col gap-2 rounded border p-2" key={valueOf(attachment.connectionId)}>
              <Row label="Attachment ID">{valueOf(attachment.connectionId)}</Row>
              <Row label="ReadModel">
                <ExactReference kind="readModel" reference={attachment.readModel} model={model} onSelect={onSelect} />
              </Row>
              {attachment.doc ? <Row label="Attachment documentation">{valueOf(attachment.doc)}</Row> : null}
            </dl>
          ))}
        </>
      )}
    </section>
  );
}

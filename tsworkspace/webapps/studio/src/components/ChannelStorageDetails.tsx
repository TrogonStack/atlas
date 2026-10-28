import { ExactReference } from './ArchitectureResourceDetails';
import {
  channelLinks,
  incomingChannelLinks,
  incomingStorageClaims,
  resolveAssociationSubject,
  resolvedAttachment,
  storageClaims,
  type AssociationView,
  type ChannelLinkView,
  type StorageClaimView,
} from '@/lib/channel-storage';
import type { Entity, Model } from '@/lib/model';
import { responsibilitySubjects } from '@/lib/responsibility';
import { contractRecord } from '@/lib/specification';

const owners = ['channel', 'component', 'externalSystem', 'dataProduct', 'dataStore'];
const isLink = (item: AssociationView): item is ChannelLinkView => 'kind' in item;

export function ChannelStorageDetails({
  entity,
  model,
  onSelect,
}: {
  entity: Entity;
  model: Model;
  onSelect: (key: string) => void;
}) {
  const owner = owners.includes(entity.kind);
  const subjects: unknown[] =
    entity.kind === 'decision' || entity.kind === 'document'
      ? Array.isArray(entity.raw.subjects)
        ? entity.raw.subjects.map((value) => contractRecord(value).subject)
        : []
      : owner
        ? []
        : responsibilitySubjects(entity)
            .slice(1)
            .map((item) => item.subject);
  const selected = subjects.flatMap((subject) => {
    const found = resolveAssociationSubject(model, subject);
    return found ? [found] : [];
  });
  if (!owner && !selected.length) return null;
  const rows = (items: AssociationView[]) =>
    items.map((item) => {
      const attachment = isLink(item) ? undefined : resolvedAttachment(model, item);
      return (
        <section
          className="flex flex-col gap-2 rounded border p-2"
          key={JSON.stringify([item.owner.key, item.owner.uid, item.connectionId, item.path])}
        >
          <p>{isLink(item) ? item.kind : item.direction} declared by the owner</p>
          <dl className="flex flex-col gap-1">
            <dt>Declaration owner</dt>
            <dd>
              <ExactReference
                kind={item.owner.kind}
                reference={{ id: item.owner.id }}
                model={model}
                onSelect={onSelect}
              />
            </dd>
            <dt>Owner UID</dt>
            <dd>{item.owner.uid || 'Unavailable'}</dd>
            <dt>Connection ID</dt>
            <dd>{item.connectionId || 'Unassigned'}</dd>
            <dt>Diagnostic path</dt>
            <dd>{item.path}</dd>
            <ReferenceRows item={item} model={model} onSelect={onSelect} attachmentExact={!!attachment} />
          </dl>
          {item.doc !== undefined && <p className="whitespace-pre-wrap">{item.doc}</p>}
        </section>
      );
    });
  const own = [...channelLinks(entity), ...storageClaims(entity)];
  const incoming = [...incomingChannelLinks(model, entity), ...incomingStorageClaims(model, entity)];
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">Channel links and storage claims</h3>
      <p>
        Owner-local authored associations. Links and claims are descriptive, and do not establish routing, delivery or
        runtime behavior. The store&apos;s access mode is not admission authority, and READ and WRITE are distinct
        declarations.
      </p>
      {owner && (
        <>
          <h4 className="font-semibold">Authored declarations</h4>
          {rows(own)}
          {!own.length && <p>No links or storage claims declared in this loaded owner.</p>}
          <h4 className="font-semibold">References to this target in the loaded model</h4>
          {rows(incoming)}
          <p>Incoming discovery is limited to loaded owners and does not reverse stored reference provenance.</p>
        </>
      )}
      {!!selected.length && (
        <>
          <h4 className="font-semibold">Selected associations in the loaded model</h4>
          {rows(selected)}
        </>
      )}
    </section>
  );
}

function ReferenceRows({
  item,
  model,
  onSelect,
  attachmentExact,
}: {
  item: AssociationView;
  model: Model;
  onSelect: (key: string) => void;
  attachmentExact: boolean;
}) {
  if (isLink(item))
    return (
      <>
        <dt>Target Channel</dt>
        <dd>
          {item.target ? (
            <ExactReference kind="channel" reference={item.target} model={model} onSelect={onSelect} />
          ) : (
            'Unavailable typed target'
          )}
        </dd>
        <dt>Parameter values</dt>
        <dd>
          {item.parameterValues.length
            ? item.parameterValues.map(([name, value]) => <span key={name}>{`${name}=${value}`}</span>)
            : 'None'}
        </dd>
      </>
    );
  const claim: StorageClaimView = item;
  return (
    <>
      <dt>Store</dt>
      <dd>
        {claim.store ? (
          <ExactReference kind="dataStore" reference={claim.store} model={model} onSelect={onSelect} />
        ) : (
          'Unavailable typed store'
        )}
      </dd>
      <dt>Attachment</dt>
      <dd>
        {claim.attachment ? (
          <>
            <ExactReference kind="dataStore" reference={claim.attachment.owner} model={model} onSelect={onSelect} />
            <span>{claim.attachment.ownerUid ?? 'Unavailable'}</span>
            <span>{claim.attachment.id ?? 'Unavailable'}</span>
            {!attachmentExact && <span>Unavailable exact attachment</span>}
          </>
        ) : (
          'None'
        )}
      </dd>
      <dt>Data port</dt>
      <dd>{claim.portId ?? 'None'}</dd>
    </>
  );
}

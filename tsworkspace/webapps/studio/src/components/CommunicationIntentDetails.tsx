import { ExactReference } from './ArchitectureResourceDetails';
import {
  communications,
  incomingCommunications,
  resolveCommunicationSubject,
  resolvedCommunicationMessage,
  type CommunicationIntent,
} from '@/lib/communication-intent';
import type { Entity, Model } from '@/lib/model';
import { responsibilitySubjects } from '@/lib/responsibility';
import { contractRecord, contractValue } from '@/lib/specification';

export function CommunicationIntentDetails({
  entity,
  model,
  onSelect,
}: {
  entity: Entity;
  model: Model;
  onSelect: (key: string) => void;
}) {
  const boundary = entity.kind === 'component' || entity.kind === 'externalSystem';
  const subjects: unknown[] =
    entity.kind === 'decision' || entity.kind === 'document'
      ? Array.isArray(entity.raw.subjects)
        ? entity.raw.subjects.map((value) => contractRecord(value).subject)
        : []
      : responsibilitySubjects(entity)
          .slice(1)
          .map((item) => item.subject);
  const selected = boundary
    ? []
    : subjects.flatMap((subject) => {
        const found = resolveCommunicationSubject(model, subject);
        return found ? [found] : [];
      });
  if (!boundary && !selected.length) return null;
  const rows = (items: CommunicationIntent[]) =>
    items.map((item) => {
      const message = resolvedCommunicationMessage(model, item);
      return (
        <section
          className="flex flex-col gap-2 rounded border p-2"
          key={JSON.stringify([item.owner.key, item.owner.uid, item.connectionId, item.path])}
        >
          <h4 className="font-semibold">{item.label}</h4>
          <p>{item.direction} relative to the declaration owner</p>
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
            <dt>Declared peer</dt>
            <dd>
              {item.peer ? (
                <ExactReference
                  kind={item.peer.kind}
                  reference={{ id: item.peer.id }}
                  model={model}
                  onSelect={onSelect}
                />
              ) : (
                'Unavailable typed peer'
              )}
            </dd>
            <dt>Selected Specification</dt>
            <dd>
              <ExactReference
                kind="specification"
                reference={item.message.specification}
                model={model}
                onSelect={onSelect}
              />
            </dd>
            <dt>Selected message member</dt>
            <dd>{contractValue(item.message.memberId)}</dd>
            <dt>Specification owner UID</dt>
            <dd>{typeof item.message.ownerUid === 'string' ? item.message.ownerUid : 'Unavailable'}</dd>
            <dt>Message in the loaded model</dt>
            <dd>{message ? message.name : 'Unavailable exact message member'}</dd>
            <dt>Condition</dt>
            <dd className="whitespace-pre-wrap">
              {item.condition ?? 'No condition documented; delivery behavior is unknown.'}
            </dd>
          </dl>
          {item.doc !== undefined && <p className="whitespace-pre-wrap">{item.doc}</p>}
        </section>
      );
    });
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">Communication intent</h3>
      <p>
        Owner-local authored intent. Conditions are descriptive prose, not evaluated predicates. Declarations do not
        establish delivery, agreement, execution or behavioral binding.
      </p>
      {boundary && (
        <>
          <h4 className="font-semibold">Authored communication declarations</h4>
          {rows(communications(entity))}
          {!communications(entity).length && (
            <p>No communications declared in this loaded owner. Operational communication remains unknown.</p>
          )}
          <h4 className="font-semibold">References to this peer in the loaded model</h4>
          {rows(incomingCommunications(model, entity))}
          <p>
            Incoming discovery is limited to loaded owners. SEND and RECEIVE do not reverse stored reference provenance.
          </p>
        </>
      )}
      {!!selected.length && (
        <>
          <h4 className="font-semibold">Selected communications in the loaded model</h4>
          {rows(selected)}
        </>
      )}
      <p>
        Loaded wording belongs to this checked owner view. Exact peer and member availability and backend subject
        resolution remain separate.
      </p>
    </section>
  );
}

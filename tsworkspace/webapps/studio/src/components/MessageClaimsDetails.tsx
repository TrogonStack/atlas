import { ExactReference } from './ArchitectureResourceDetails';
import {
  displaySelector,
  incomingMessageClaims,
  localClaim,
  messageClaimLeaves,
  resolveMessageClaimSubject,
  resolveMessageMember,
  resolvePayloadBinding,
  type MessageClaimLeaf,
} from '@/lib/message-claims';
import { wireKind, type Entity, type Model } from '@/lib/model';
import { responsibilitySubjects } from '@/lib/responsibility';
import { contractRecord as record, contractValue } from '@/lib/specification';

const heading: Record<MessageClaimLeaf['list'], string> = {
  claims: 'Claim',
  routes: 'Route',
  triggers: 'Trigger',
  fieldSelections: 'Field selection',
};
const directionText = (value: unknown) =>
  value === 'COMMUNICATION_DIRECTION_SEND'
    ? 'SEND'
    : value === 'COMMUNICATION_DIRECTION_RECEIVE'
      ? 'RECEIVE'
      : 'Unavailable';

type Props = { model: Model; onSelect: (key: string) => void };

function Row({ term, children }: { term: string; children: React.ReactNode }) {
  return (
    <>
      <dt>{term}</dt>
      <dd>{children}</dd>
    </>
  );
}

function MessageRows({ selection, model, onSelect }: { selection: Record<string, unknown> } & Props) {
  const message = resolveMessageMember(model, selection);
  return (
    <>
      <Row term="Selected Specification">
        <ExactReference kind="specification" reference={selection.specification} model={model} onSelect={onSelect} />
      </Row>
      <Row term="Selected message member">{contractValue(selection.memberId)}</Row>
      <Row term="Specification owner UID">
        {typeof selection.ownerUid === 'string' ? selection.ownerUid : 'Unavailable'}
      </Row>
      <Row term="Message in the loaded model">{message ? message.name : 'Unavailable exact message member'}</Row>
    </>
  );
}

function LocalClaim({ leaf }: { leaf: MessageClaimLeaf }) {
  const claim = localClaim(leaf);
  const id = record(leaf.body.claimId).value;
  return (
    <>
      <Row term="Claim ID">{typeof id === 'string' && id ? id : 'Unavailable'}</Row>
      <Row term="Claim in this owner">
        {claim === 'ambiguous' ? 'Ambiguous claim ID' : claim ? claim.path : 'Unavailable exact claim'}
      </Row>
    </>
  );
}

function Leaf({ leaf, model, onSelect }: { leaf: MessageClaimLeaf } & Props) {
  const { body } = leaf;
  const params = Object.entries(record(body.parameterValues)).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  const binding = record(body.binding);
  const bindingKind = typeof record(binding.owner).kind === 'string' ? wireKind(record(binding.owner).kind) : undefined;
  const selectors = Array.isArray(body.selectors) ? body.selectors : [];
  const delivery =
    typeof body.deliveryMode === 'string' ? body.deliveryMode.replace('CLAIM_DELIVERY_MODE_', '') : undefined;
  return (
    <section className="flex flex-col gap-2 rounded border p-2">
      <h4 className="font-semibold">
        {heading[leaf.list]} {leaf.connectionId ?? 'without ID'}
      </h4>
      {leaf.list === 'claims' && <p>{directionText(body.direction)} relative to the declaration owner</p>}
      <dl className="flex flex-col gap-1">
        <Row term="Declaration owner">
          <ExactReference kind={leaf.owner.kind} reference={{ id: leaf.owner.id }} model={model} onSelect={onSelect} />
        </Row>
        <Row term="Owner UID">{leaf.owner.uid || 'Unavailable'}</Row>
        <Row term="Connection ID">{leaf.connectionId || 'Unassigned'}</Row>
        <Row term="Diagnostic path">{leaf.path}</Row>
        {leaf.list !== 'claims' && <LocalClaim leaf={leaf} />}
        {(leaf.list === 'claims' || leaf.list === 'triggers') && (
          <MessageRows selection={record(body.message)} model={model} onSelect={onSelect} />
        )}
        {leaf.list === 'claims' && <Row term="Group">{contractValue(body.group)}</Row>}
        {leaf.list === 'routes' && (
          <>
            <Row term="Channel">
              <ExactReference kind="channel" reference={body.channel} model={model} onSelect={onSelect} />
            </Row>
            <Row term="Channel parameter values">
              {params.length ? (
                <ul className="flex flex-col gap-1">
                  {params.map(([name, value]) => (
                    <li key={name}>{`${name}: ${JSON.stringify(value)}`}</li>
                  ))}
                </ul>
              ) : (
                'None declared'
              )}
            </Row>
            <Row term="Delivery mode">{delivery ?? 'Undeclared'}</Row>
          </>
        )}
        {leaf.list === 'triggers' && (
          <Row term="Condition">
            <span className="whitespace-pre-wrap">
              {record(body.condition).value
                ? contractValue(body.condition)
                : 'No condition documented; delivery behavior is unknown.'}
            </span>
          </Row>
        )}
        {leaf.list === 'fieldSelections' && (
          <>
            <Row term="Binding owner">
              {bindingKind === 'specification' ? (
                <ExactReference kind="specification" reference={binding.owner} model={model} onSelect={onSelect} />
              ) : (
                'Unavailable exact binding owner'
              )}
            </Row>
            <Row term="Binding owner UID">
              {typeof binding.ownerUid === 'string' ? binding.ownerUid : 'Unavailable'}
            </Row>
            <Row term="Binding ID">{contractValue(binding.id)}</Row>
            <Row term="PAYLOAD binding">
              {resolvePayloadBinding(model, binding)
                ? 'PAYLOAD binding available in the loaded model'
                : 'Unavailable exact PAYLOAD binding'}
            </Row>
            <Row term="Selectors">
              <ul className="flex flex-col gap-1">
                {selectors.map((value, index) => {
                  const selector = displaySelector(value);
                  return (
                    <li
                      key={`${index}:${selector.kind}:${selector.kind === 'unavailable' ? '' : selector.text}`}
                      className="flex flex-col gap-1"
                    >
                      {selector.kind === 'path' && (
                        <>
                          <code>{selector.text}</code>
                          <span>Path as authored; Studio does not verify it</span>
                        </>
                      )}
                      {selector.kind === 'opaque' && (
                        <>
                          <code>{selector.text}</code>
                          <span>Opaque selector, unverified</span>
                        </>
                      )}
                      {selector.kind === 'unavailable' && <span>Unavailable selector</span>}
                    </li>
                  );
                })}
                {!selectors.length && <li>No selectors declared</li>}
              </ul>
            </Row>
          </>
        )}
        {body.doc !== undefined && <Row term="Documentation">{contractValue(body.doc)}</Row>}
      </dl>
    </section>
  );
}

export function MessageClaimsDetails({ entity, model, onSelect }: { entity: Entity } & Props) {
  const owner = entity.kind === 'component' || entity.kind === 'externalSystem' || entity.kind === 'domain';
  const target = entity.kind === 'channel' || entity.kind === 'specification';
  const subjects: unknown[] =
    entity.kind === 'decision' || entity.kind === 'document'
      ? Array.isArray(entity.raw.subjects)
        ? entity.raw.subjects.map((value) => record(value).subject)
        : []
      : responsibilitySubjects(entity)
          .slice(1)
          .map((item) => item.subject);
  const selected = owner
    ? []
    : subjects.flatMap((subject) => {
        const found = resolveMessageClaimSubject(model, subject);
        return found ? [found] : [];
      });
  if (!owner && !target && !selected.length) return null;
  const rows = (items: MessageClaimLeaf[]) => {
    const groups = new Map<string, { label?: string; leaves: MessageClaimLeaf[] }>();
    for (const leaf of items) {
      const label = record(leaf.body.group).value;
      const grouped = leaf.list === 'claims' && typeof label === 'string' && label.trim().length > 0;
      const key = JSON.stringify([
        leaf.owner.key,
        leaf.owner.uid,
        grouped ? leaf.body.direction : leaf.path,
        grouped ? label : leaf.connectionId,
      ]);
      const group = groups.get(key);
      if (group) group.leaves.push(leaf);
      else
        groups.set(key, {
          label: grouped ? `${directionText(leaf.body.direction)} group ${label}` : undefined,
          leaves: [leaf],
        });
    }
    const renderLeaf = (leaf: MessageClaimLeaf) => (
      <Leaf
        key={JSON.stringify([leaf.owner.key, leaf.owner.uid, leaf.connectionId, leaf.path])}
        leaf={leaf}
        model={model}
        onSelect={onSelect}
      />
    );
    return [...groups.entries()].map(([key, group]) =>
      group.label ? (
        <section key={key} aria-label={group.label} className="flex flex-col gap-2">
          <h5 className="font-semibold">{group.label}</h5>
          {group.leaves.map(renderLeaf)}
        </section>
      ) : (
        group.leaves.map(renderLeaf)
      ),
    );
  };
  const authored = owner ? messageClaimLeaves(entity) : [];
  const incoming = target ? incomingMessageClaims(model, entity) : [];
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">Message claims</h3>
      <p>
        Owner-local authored declarations. A claim, route, trigger or field selection stands alone: its presence is not
        evidence of execution, delivery or member rollup, and equal siblings are never merged.
      </p>
      {owner && (
        <>
          <h4 className="font-semibold">Authored message claims</h4>
          {rows(authored)}
          {!authored.length && <p>No message claims declared in this loaded owner.</p>}
        </>
      )}
      {target && (
        <>
          <h4 className="font-semibold">Claims that name this {entity.kind} in the loaded model</h4>
          {rows(incoming)}
          {!incoming.length && <p>No loaded owner declares a claim that names this {entity.kind}.</p>}
          <p>Incoming discovery is limited to loaded owners. Each entry is matched on its own target only.</p>
        </>
      )}
      {!!selected.length && (
        <>
          <h4 className="font-semibold">Selected message claims in the loaded model</h4>
          {rows(selected)}
        </>
      )}
    </section>
  );
}

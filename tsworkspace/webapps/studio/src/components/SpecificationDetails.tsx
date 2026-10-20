import type { ReactNode } from 'react';
import { ExactReference } from '@/components/ArchitectureResourceDetails';
import { type Entity, type Model } from '@/lib/model';
import {
  contractList as list,
  contractRecord as record,
  contractValue as value,
  contractMember,
  isApiSpecification,
  resolveContractType,
  type ContractRecord,
} from '@/lib/specification';

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="break-words">{children}</dd>
    </div>
  );
}
function Occurrence({ item, children }: { item: ContractRecord; children: ReactNode }) {
  return (
    <dl className="flex flex-col gap-2 rounded border p-2">
      <Row label="Association ID">{value(item.connectionId)}</Row>
      {children}
      {item.doc !== undefined ? <Row label="Documentation">{value(item.doc)}</Row> : null}
    </dl>
  );
}
function Member({ item, children }: { item: ContractRecord; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2 rounded border p-2">
      <h4 className="font-semibold">{value(item.name)}</h4>
      <dl>
        <Row label="Member ID">{value(item.memberId)}</Row>
        {item.doc !== undefined ? <Row label="Documentation">{value(item.doc)}</Row> : null}
      </dl>
      {children}
    </section>
  );
}
function LocalMember({ owner, collection, memberId }: { owner: Entity; collection: string; memberId: unknown }) {
  const member = contractMember(owner, collection, memberId);
  return (
    <span>
      {value(memberId)}
      {member ? ` (${value(member.name)})` : ' (unresolved member)'}
    </span>
  );
}
function Selector({ selector }: { selector: unknown }) {
  return (
    <Row label="Opaque selector">
      {selector === undefined
        ? 'Whole artifact (unverified)'
        : `${value(record(selector).syntax)}: ${value(record(selector).value)} (unverified)`}
    </Row>
  );
}

export function SpecificationDetails({
  entity,
  model,
  onSelect,
}: {
  entity: Entity;
  model: Model;
  onSelect: (key: string) => void;
}) {
  if (entity.kind !== 'specification') return null;
  const raw = entity.raw;
  const reference = (kind: Parameters<typeof ExactReference>[0]['kind'], target: unknown) => (
    <ExactReference kind={kind} reference={target} model={model} onSelect={onSelect} />
  );
  const provider = (target: unknown) => {
    const p = record(target);
    return p.component
      ? reference('component', p.component)
      : p.externalSystem
        ? reference('externalSystem', p.externalSystem)
        : 'Undeclared';
  };
  const schemaType = (binding: ContractRecord) => {
    if (binding.localSchema) return <LocalMember owner={entity} collection="schemas" memberId={binding.localSchema} />;
    if (binding.schema) return reference('schema', binding.schema);
    if (binding.libraryType)
      return (
        <>
          {reference('typeLibrary', record(binding.libraryType).library)}: {value(record(binding.libraryType).message)}
        </>
      );
    if (binding.contractType) {
      const selection = record(binding.contractType);
      const ref = record(selection.schema);
      const resolved = resolveContractType(model, selection);
      return (
        <div className="flex flex-col gap-1">
          {reference('specification', ref.specification)}
          <span>Owner UID: {value(ref.ownerUid)}</span>
          <span>Schema member: {value(ref.memberId)}</span>
          <span>{resolved ? value(resolved.name) : 'Not resolved in the loaded model'}</span>
          <span>Artifact member: {value(selection.expectedArtifactId)}</span>
          <span>SHA-256: {value(record(selection.expectedDigest).sha256)}</span>
          <dl>
            <Selector selector={selection.expectedSelector} />
          </dl>
        </div>
      );
    }
    return 'Undeclared';
  };
  return (
    <section className="flex flex-col gap-3" aria-label="Specification declaration">
      <h3 className="text-xs font-semibold uppercase text-muted-foreground">
        {isApiSpecification(entity) ? 'API specification' : 'Specification'}
      </h3>
      <p className="text-xs text-muted-foreground">
        Declarations describe contracts and dependencies. They do not establish runtime traffic or execution.
      </p>
      <h3 className="font-semibold">Retained artifacts</h3>
      {list(raw.artifacts).map((artifact) => (
        <Member key={value(artifact.memberId)} item={artifact}>
          <dl className="flex flex-col gap-2">
            <Row label="Path">{value(artifact.path)}</Row>
            <Row label="Format">
              {record(artifact.format).other !== undefined
                ? value(record(artifact.format).other)
                : value(record(artifact.format).known).replace('KNOWN_', '')}
            </Row>
            <Row label="Format version">{value(artifact.formatVersion)}</Row>
            <Row label="Media type">{value(artifact.mediaType)}</Row>
            <Row label="SHA-256">{value(record(artifact.digest).sha256)}</Row>
          </dl>
          <details>
            <summary>Retained source text</summary>
            <pre className="overflow-auto whitespace-pre-wrap text-xs">{value(artifact.content)}</pre>
          </details>
        </Member>
      ))}
      <h3 className="font-semibold">Schemas</h3>
      {list(raw.schemas).map((schema) => {
        const definition = record(schema.definition);
        return (
          <Member key={value(schema.memberId)} item={schema}>
            <Occurrence item={definition}>
              {definition.artifact ? (
                <>
                  <Row label="Artifact">
                    <LocalMember
                      owner={entity}
                      collection="artifacts"
                      memberId={record(definition.artifact).artifactId}
                    />
                  </Row>
                  <Row label="Expected SHA-256">{value(record(record(definition.artifact).expectedDigest).sha256)}</Row>
                  <Selector selector={definition.selector} />
                </>
              ) : (
                <Row label="Native schema">{reference('schema', definition.schema)}</Row>
              )}
            </Occurrence>
          </Member>
        );
      })}
      <h3 className="font-semibold">Messages</h3>
      {list(raw.messages).map((message) => (
        <Member key={value(message.memberId)} item={message}>
          {message.mapping ? (
            <Occurrence item={record(message.mapping)}>
              <Row label="ReadModel mapping">{reference('readModel', record(message.mapping).readModel)}</Row>
            </Occurrence>
          ) : (
            <p>ReadModel mapping undeclared</p>
          )}
          {!list(message.schemas).length ? <p>Message shape undeclared</p> : null}
          {list(message.schemas).map((binding) => (
            <Occurrence key={value(binding.connectionId)} item={binding}>
              <Row label="Schema role">{value(binding.role).replace('ROLE_', '')}</Row>
              <Row label="Schema type">{schemaType(binding)}</Row>
            </Occurrence>
          ))}
        </Member>
      ))}
      <h3 className="font-semibold">Operations</h3>
      {list(raw.operations).map((operation) => {
        const binding = record(operation.binding);
        return (
          <Member key={value(operation.memberId)} item={operation}>
            <dl>
              {binding.http ? (
                <Row label="HTTP">
                  {value(record(binding.http).method)} {value(record(binding.http).path)}
                  {Array.isArray(record(binding.http).responseStatusCodes)
                    ? ` (${(record(binding.http).responseStatusCodes as unknown[]).join(', ')})`
                    : ''}
                </Row>
              ) : binding.rpc ? (
                <Row label="RPC">
                  {value(record(binding.rpc).service)} / {value(record(binding.rpc).method)}
                </Row>
              ) : binding.graphql ? (
                <Row label="GraphQL">
                  {value(record(binding.graphql).kind).replace('KIND_', '')} {value(record(binding.graphql).field)}
                </Row>
              ) : binding.other ? (
                <Row label="Transport">
                  {value(record(binding.other).protocol)}: {value(record(binding.other).description)}
                </Row>
              ) : !binding.channel ? (
                <Row label="Transport">Undeclared</Row>
              ) : null}
            </dl>
            {binding.channel ? (
              <Occurrence item={record(binding.channel)}>
                <Row label="Channel">{reference('channel', record(binding.channel).channel)}</Row>
              </Occurrence>
            ) : null}
            {list(operation.messages).map((occurrence) => (
              <Occurrence key={value(occurrence.connectionId)} item={occurrence}>
                <Row label="Message role">{value(occurrence.role).replace('ROLE_', '')}</Row>
                <Row label="Message">
                  <LocalMember owner={entity} collection="messages" memberId={occurrence.messageId} />
                </Row>
              </Occurrence>
            ))}
          </Member>
        );
      })}
      <h3 className="font-semibold">Provider exposures</h3>
      {!list(raw.exposures).length ? <p>Provider undeclared</p> : null}
      {list(raw.exposures).map((exposure) => (
        <Occurrence key={value(exposure.connectionId)} item={exposure}>
          <Row label="Provider">{provider(exposure.provider)}</Row>
          <Row label="Operation">
            <LocalMember owner={entity} collection="operations" memberId={exposure.operationId} />
          </Row>
        </Occurrence>
      ))}
      <h3 className="font-semibold">API usages</h3>
      {list(raw.usages).map((usage) => {
        const exposure = list(raw.exposures).find(
          (candidate) =>
            value(candidate.connectionId) === value(usage.exposureId) &&
            value(candidate.operationId) === value(usage.operationId),
        );
        return (
          <Occurrence key={value(usage.connectionId)} item={usage}>
            <Row label="Caller">{provider(usage.caller)}</Row>
            <Row label="Operation">
              <LocalMember owner={entity} collection="operations" memberId={usage.operationId} />
            </Row>
            <Row label="Selected exposure">{usage.exposureId ? value(usage.exposureId) : 'Provider undeclared'}</Row>
            {usage.exposureId ? (
              <Row label="Selected provider">{exposure ? provider(exposure.provider) : 'Unresolved exposure'}</Row>
            ) : null}
          </Occurrence>
        );
      })}
      <h3 className="font-semibold">Dependencies</h3>
      {list(raw.dependencies).map((dependency) => (
        <Occurrence key={value(dependency.connectionId)} item={dependency}>
          <Row label="Contract">
            {dependency.specification
              ? reference('specification', dependency.specification)
              : reference('typeLibrary', dependency.typeLibrary)}
          </Row>
        </Occurrence>
      ))}
    </section>
  );
}

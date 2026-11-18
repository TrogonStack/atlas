import { ExactReference } from './ArchitectureResourceDetails';
import {
  architecturalDeclarations,
  incomingArchitecturalDeclarations,
  resolveArchitecturalSubject,
  type ArchitecturalDeclaration,
} from '@/lib/architectural-relationships';
import type { Entity, Model } from '@/lib/model';
import { responsibilitySubjects } from '@/lib/responsibility';

function Declaration({
  declaration,
  model,
  onSelect,
}: {
  declaration: ArchitecturalDeclaration;
  model: Model;
  onSelect: (key: string) => void;
}) {
  const { owner, target, connectionId, label, doc, path } = declaration;
  return (
    <section className="flex flex-col gap-2 rounded border p-2">
      <h4 className="font-semibold">{label}</h4>
      {doc !== undefined && <p className="whitespace-pre-wrap">{doc}</p>}
      <dl className="flex flex-col gap-1">
        <dt>Declaration owner</dt>
        <dd>
          <ExactReference kind={owner.kind} reference={{ id: owner.id }} model={model} onSelect={onSelect} />
        </dd>
        <dt>Owner UID</dt>
        <dd className="break-all">{owner.uid || 'Unavailable'}</dd>
        <dt>Connection ID</dt>
        <dd className="break-all">{connectionId || 'Unassigned'}</dd>
        <dt>Diagnostic path</dt>
        <dd>{path}</dd>
        <dt>Declared target</dt>
        <dd>
          {target ? (
            <ExactReference kind={target.kind} reference={{ id: target.id }} model={model} onSelect={onSelect} />
          ) : (
            'Unavailable typed target'
          )}
        </dd>
      </dl>
    </section>
  );
}
export function ArchitecturalRelationshipDetails({
  entity,
  model,
  onSelect,
}: {
  entity: Entity;
  model: Model;
  onSelect: (key: string) => void;
}) {
  const boundary = entity.kind === 'component' || entity.kind === 'externalSystem';
  const authored = boundary ? architecturalDeclarations(entity) : [];
  const incoming = boundary ? incomingArchitecturalDeclarations(model, entity) : [];
  const subjects =
    entity.kind === 'decision' || entity.kind === 'document'
      ? Array.isArray(entity.raw.subjects)
        ? entity.raw.subjects.flatMap((value) => {
            const subject = (value as { subject?: Parameters<typeof resolveArchitecturalSubject>[1] })?.subject;
            return subject ? [subject] : [];
          })
        : []
      : responsibilitySubjects(entity)
          .slice(1)
          .map((item) => item.subject);
  const selected = boundary
    ? []
    : subjects.flatMap((subject) => {
        const declaration = resolveArchitecturalSubject(model, subject);
        return declaration ? [declaration] : [];
      });
  if (!boundary && !selected.length) return null;
  const rows = (values: ArchitecturalDeclaration[]) =>
    values.map((declaration) => (
      <Declaration
        key={JSON.stringify([declaration.owner.key, declaration.owner.uid, declaration.connectionId, declaration.path])}
        declaration={declaration}
        model={model}
        onSelect={onSelect}
      />
    ));
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">Architectural declarations</h3>
      <p>
        Directed authored intent. Labels establish no message flow, execution, containment, ownership or human
        interaction.
      </p>
      {boundary && (
        <>
          <h4 className="font-semibold">Authored outbound declarations</h4>
          {rows(authored)}
          {!authored.length && (
            <p>No authored outbound declarations in this loaded owner. Operational dependencies remain unknown.</p>
          )}
          <h4 className="font-semibold">Incoming declarations in the loaded model</h4>
          {rows(incoming)}
          <p>Incoming discovery is limited to loaded owners. Missing records do not establish complete absence.</p>
        </>
      )}
      {!!selected.length && (
        <>
          <h4 className="font-semibold">Selected declarations in the loaded model</h4>
          {rows(selected)}
        </>
      )}
      <p>
        Exact owner UID and connection ID identify an occurrence. Loaded wording is current only for this checked view;
        target availability and backend subject resolution are separate.
      </p>
    </section>
  );
}

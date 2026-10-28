import { ExactReference } from './ArchitectureResourceDetails';
import { groupEdges, type GroupEdge } from '@/lib/system-group';
import type { Entity, Model } from '@/lib/model';

const SECTION_TITLES: Record<GroupEdge['section'], string> = {
  members: 'Members',
  relationships: 'Relationships to other groups',
  actors: 'Actors',
};

function directionLabel(direction: string | undefined): string {
  if (!direction) return 'Undeclared';
  return direction.replace('SYSTEM_ACTOR_DIRECTION_', '').toLowerCase();
}

function Edge({ edge, model, onSelect }: { edge: GroupEdge; model: Model; onSelect: (key: string) => void }) {
  const { target } = edge;
  return (
    <section className="flex flex-col gap-1 rounded border p-2">
      {edge.label !== undefined && <h5 className="font-semibold">{edge.label}</h5>}
      {edge.doc !== undefined && <p className="whitespace-pre-wrap">{edge.doc}</p>}
      <dl className="flex flex-col gap-1">
        <dt>Diagnostic path</dt>
        <dd>{edge.path}</dd>
        <dt>Connection ID</dt>
        <dd className="break-all">{edge.connectionId || 'Unassigned'}</dd>
        {edge.section === 'actors' && (
          <>
            <dt>Direction</dt>
            <dd>{directionLabel(edge.direction)}</dd>
          </>
        )}
        <dt>Declared target</dt>
        <dd>
          {target.kind && target.id ? (
            <ExactReference kind={target.kind} reference={{ id: target.id }} model={model} onSelect={onSelect} />
          ) : (
            `Unavailable typed target${target.wire ? ` (${target.wire})` : ''}`
          )}
        </dd>
      </dl>
    </section>
  );
}

export function SystemGroupDetails({
  entity,
  model,
  onSelect,
}: {
  entity: Entity;
  model: Model;
  onSelect: (key: string) => void;
}) {
  const edges = groupEdges(entity);
  if (entity.kind !== 'systemGroup' && !edges.length) return null;
  const sections = (['members', 'relationships', 'actors'] as const).filter(
    (section) => entity.kind === 'systemGroup' || section === 'members',
  );
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">{entity.kind === 'systemGroup' ? 'System group curation' : 'Domain members'}</h3>
      <p>
        Authored curation only. Membership, relationships and actors establish no ownership, authorization, runtime
        behavior or message flow.
      </p>
      {entity.kind === 'systemGroup' && (
        <dl className="flex flex-col gap-1">
          <dt>Scope</dt>
          <dd>
            {String(entity.raw.scope ?? 'Undeclared')
              .replace('SYSTEM_GROUP_SCOPE_', '')
              .toLowerCase()}
          </dd>
          <dt>Owner UID</dt>
          <dd className="break-all">{entity.uid || 'Unavailable'}</dd>
        </dl>
      )}
      {sections.map((section) => {
        const rows = edges.filter((edge) => edge.section === section);
        return (
          <div key={section} className="flex flex-col gap-2">
            <h4 className="font-semibold">{SECTION_TITLES[section]}</h4>
            {rows.map((edge) => (
              <Edge key={`${edge.path}:${edge.connectionId}`} edge={edge} model={model} onSelect={onSelect} />
            ))}
            {!rows.length && <p>None authored in this loaded owner.</p>}
          </div>
        );
      })}
      <p>
        Exact owner UID and connection ID identify an occurrence. Equal targets stay distinct and cycles remain visible;
        target availability is checked separately.
      </p>
    </section>
  );
}

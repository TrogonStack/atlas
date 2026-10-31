import { ExactReference } from './ArchitectureResourceDetails';
import { uiHostings } from '@/lib/ui-hosting';
import type { Entity, Model } from '@/lib/model';

export function UiHostingDetails({ entity, model, onSelect }: { entity: Entity; model: Model; onSelect: (key: string) => void }) {
  if (entity.kind !== 'component') return null;
  const hostings = uiHostings(entity);
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">UI hosting and human interaction</h3>
      <p>Authored UI hosting assertions do not establish deployment, permissions or runtime execution.</p>
      {!hostings.length && <p>No UI hosting assertions in this loaded owner.</p>}
      {hostings.map((item) => (
        <section className="flex flex-col gap-2 rounded border p-2" key={JSON.stringify([entity.uid, item.path, item.connectionId])}>
          <h4 className="font-semibold">{item.label ?? 'Unavailable label'}</h4>
          <dl className="flex flex-col gap-1">
            <dt>Owner UID</dt><dd>{entity.uid || 'Unavailable'}</dd>
            <dt>Connection ID</dt><dd>{item.connectionId || 'Unassigned'}</dd>
            <dt>Diagnostic path</dt><dd>{item.path}</dd>
            <dt>Hosted UI</dt><dd>{item.ui ? <ExactReference kind="ui" reference={item.ui} model={model} onSelect={onSelect} /> : 'Unavailable typed UI'}</dd>
          </dl>
          {item.doc !== undefined && <p className="whitespace-pre-wrap">{item.doc}</p>}
        </section>
      ))}
      {!!hostings.length && <p>Select an exact hosting and a complete slice tuple before checking an interaction. Loaded memberships do not establish a human path.</p>}
    </section>
  );
}

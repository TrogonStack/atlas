import { ArrowDown, X } from 'lucide-react';
import { CopyContextButton } from '@/components/CopyContextButton';
import { kindStyle } from '@/components/canvas/StickyNode';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { edgeContext } from '@/lib/context';
import type { BoardEdgeData } from '@/lib/layout';
import type { Entity } from '@/lib/model';

function EndpointButton({ entity, onSelect }: { entity: Entity; onSelect: (key: string) => void }) {
  const style = kindStyle(entity.kind);
  return (
    <button
      type="button"
      className="w-full rounded-md border border-border px-2 py-1.5 text-left text-xs hover:bg-accent"
      onClick={() => onSelect(entity.key)}
    >
      <Badge variant="secondary" className="mr-2">
        {style.label}
      </Badge>
      <span className="font-medium">{entity.title}</span>
    </button>
  );
}

export function EdgeInspector({
  edge,
  onClose,
  onSelect,
}: {
  edge: BoardEdgeData;
  onClose: () => void;
  onSelect: (key: string) => void;
}) {
  // React Flow `edge.data` is loosely typed at the canvas boundary; treat
  // missing relation/metadata as empty rather than crashing the drawer.
  const relation = edge.relation || 'connection';
  const metadata = edge.metadata ?? [];
  return (
    <aside className="flex h-full w-80 flex-col border-l border-border bg-card">
      <div className="flex items-start gap-2 p-4">
        <div>
          <Badge variant="secondary">Connection</Badge>
          <h2 className="mt-2 text-base font-semibold capitalize leading-tight">{relation}</h2>
        </div>
        <Button variant="ghost" size="icon" className="ml-auto" onClick={onClose}>
          <X />
        </Button>
      </div>
      <div className="px-4 pb-3">
        <CopyContextButton getText={() => edgeContext(edge)} />
      </div>
      <Separator />
      <div className="flex-1 space-y-4 overflow-y-auto p-4 text-sm">
        <section className="space-y-1">
          <EndpointButton entity={edge.source} onSelect={onSelect} />
          <div className="flex items-center justify-center text-muted-foreground">
            <ArrowDown className="h-3.5 w-3.5" />
            <span className="ml-1 text-[10px] uppercase">{relation}</span>
          </div>
          <EndpointButton entity={edge.target} onSelect={onSelect} />
        </section>
        {edge.doc ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Doc</h3>
            <p className="leading-relaxed text-foreground/90">{edge.doc}</p>
          </section>
        ) : null}
        {metadata.length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Metadata ({metadata.length})</h3>
            <div className="space-y-1">
              {metadata.map((m, i) => (
                <pre
                  // biome-ignore lint/suspicious/noArrayIndexKey: annotations have no stable id
                  key={i}
                  className="overflow-x-auto rounded-md border border-border bg-muted/40 p-2 text-[10px]"
                >
                  {JSON.stringify(m, null, 2)}
                </pre>
              ))}
            </div>
          </section>
        ) : null}
        {!edge.doc && metadata.length === 0 ? (
          <p className="text-xs text-muted-foreground">
            No design data is attached to this connection yet. The model supports a doc and typed annotations on every
            edge.
          </p>
        ) : null}
        {edge.slice ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">
              {relation.startsWith('navigates') || relation.startsWith('hands off') ? 'Derived from' : 'Declared by'}
            </h3>
            <EndpointButton entity={edge.slice} onSelect={onSelect} />
          </section>
        ) : null}
      </div>
    </aside>
  );
}

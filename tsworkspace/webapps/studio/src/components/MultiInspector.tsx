import { Spline, X } from 'lucide-react';
import { CopyContextButton } from '@/components/CopyContextButton';
import type { SelectedEdge } from '@/components/canvas/Board';
import { kindStyle } from '@/components/canvas/StickyNode';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { edgeContext, entityContext } from '@/lib/context';
import { EMPTY_ISSUE_INDEX, type IssueIndex } from '@/lib/issues';
import type { Entity, Model } from '@/lib/model';

export function MultiInspector({
  model,
  entities,
  edges,
  issues = EMPTY_ISSUE_INDEX,
  onClose,
  onSelect,
  onSelectEdge,
}: {
  model: Model;
  entities: Entity[];
  edges: SelectedEdge[];
  /** Validator findings for the whole model; each entity contributes its own. */
  issues?: IssueIndex;
  onClose: () => void;
  onSelect: (key: string) => void;
  onSelectEdge: (edge: SelectedEdge) => void;
}) {
  const total = entities.length + edges.length;
  const bundle = () =>
    [
      `Context bundle: ${total} items from an Event Modeling board.`,
      '',
      [
        ...entities.map((e) => entityContext(model, e, undefined, issues.for(e))),
        ...edges.map((e) => edgeContext(e.data)),
      ].join('\n\n---\n\n'),
    ].join('\n');
  return (
    <aside className="flex h-full w-80 flex-col border-l border-border bg-card">
      <div className="flex items-start gap-2 p-4">
        <div>
          <Badge variant="secondary">{total} selected</Badge>
          <p className="mt-2 text-[11px] text-muted-foreground">
            ⌘-click (or Shift-click) stickies and connections to add or remove them. Copy context bundles everything.
          </p>
        </div>
        <Button variant="ghost" size="icon" className="ml-auto" onClick={onClose}>
          <X />
        </Button>
      </div>
      <div className="px-4 pb-3">
        <CopyContextButton getText={bundle} />
      </div>
      <Separator />
      <div className="flex-1 space-y-1 overflow-y-auto p-4 text-sm">
        {entities.map((e) => (
          <button
            key={e.key}
            type="button"
            className="flex w-full items-center gap-2 rounded-md border border-border px-2 py-1.5 text-left text-xs hover:bg-accent"
            onClick={() => onSelect(e.key)}
            title="Click to focus this entity"
          >
            <Badge variant="secondary">{kindStyle(e.kind).label}</Badge>
            <span className="truncate font-medium">{e.title}</span>
          </button>
        ))}
        {edges.map((e) => (
          <button
            key={e.id}
            type="button"
            className="flex w-full items-center gap-2 rounded-md border border-border px-2 py-1.5 text-left text-xs hover:bg-accent"
            onClick={() => onSelectEdge(e)}
            title="Click to focus this connection"
          >
            <Spline className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <span className="truncate">
              {e.data.source?.title ?? '?'} <span className="text-muted-foreground">{e.data.relation || '→'}</span>{' '}
              {e.data.target?.title ?? '?'}
            </span>
          </button>
        ))}
      </div>
    </aside>
  );
}

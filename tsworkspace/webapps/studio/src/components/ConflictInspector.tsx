// CONFLICT VIEW (Phase 3: Studio), a read-only review surface for a
// branch's diff. Mirrors Inspector.tsx's <aside> drawer idiom rather than
// inventing a new modal system (there is no headless-UI/dialog library in
// this codebase). Reachable from BranchBar's "Review" button.
//
// Entries are grouped by status; selecting a conflicted entry shows base /
// ours / theirs as pretty JSON with the conflicting field paths listed
// above. Resolution stays in trogon-atlas/MCP for now; this view never writes.
import { X } from 'lucide-react';
import { useMemo, useState } from 'react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import type { BranchDiffEntry } from '@/lib/api';
import { type BranchStatus, branchEntryStatus } from '@/lib/branch';
import { cn } from '@/lib/utils';

const STATUS_LABEL: Record<BranchStatus, string> = {
  added: 'Added',
  changed: 'Changed',
  deleted: 'Deleted',
  conflict: 'Conflict',
};

const STATUS_BADGE_CLASS: Record<BranchStatus, string> = {
  added: 'border-emerald-400 bg-emerald-100 text-emerald-800',
  changed: 'border-sky-400 bg-sky-100 text-sky-800',
  deleted: 'border-zinc-400 bg-zinc-100 text-zinc-700',
  conflict: 'border-red-400 bg-red-100 text-red-800',
};

function entryLabel(entry: BranchDiffEntry): string {
  const id = entry.ref?.id;
  if (!id?.namespace || !id?.slug) return '(unresolvable ref)';
  const kind = String(entry.ref?.kind ?? '')
    .replace('ENTITY_KIND_', '')
    .toLowerCase();
  return `${kind ? `${kind} ` : ''}${id.namespace}/${id.slug}`;
}

function entryKey(entry: BranchDiffEntry, i: number): string {
  const id = entry.ref?.id;
  return id?.namespace && id?.slug ? `${entry.ref?.kind}:${id.namespace}/${id.slug}@${id.version ?? ''}` : String(i);
}

function JsonPane({ title, value }: { title: string; value: unknown }) {
  return (
    <div className="min-w-0 flex-1">
      <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">{title}</div>
      <pre className="max-h-64 overflow-auto rounded-md border border-border bg-muted/40 p-2 font-mono text-[10px] leading-snug">
        {value == null ? '(absent)' : JSON.stringify(value, null, 2)}
      </pre>
    </div>
  );
}

export function ConflictInspector({
  branch,
  entries,
  onClose,
}: {
  branch: string;
  entries: BranchDiffEntry[];
  onClose: () => void;
}) {
  const [selectedIdx, setSelectedIdx] = useState<number | undefined>(undefined);

  const grouped = useMemo(() => {
    const groups: Record<BranchStatus, { entry: BranchDiffEntry; idx: number }[]> = {
      conflict: [],
      changed: [],
      added: [],
      deleted: [],
    };
    entries.forEach((entry, idx) => {
      const status = branchEntryStatus(entry.status);
      if (status) groups[status].push({ entry, idx });
    });
    return groups;
  }, [entries]);

  const selected = selectedIdx != null ? entries[selectedIdx] : undefined;
  const selectedStatus = selected ? branchEntryStatus(selected.status) : undefined;

  return (
    <aside className="flex h-full w-[28rem] flex-col border-l border-border bg-card">
      <div className="flex items-start gap-2 p-4">
        <div>
          <div className="flex items-center gap-2">
            <Badge variant="secondary">Branch review</Badge>
            <span className="font-mono text-[11px] text-muted-foreground">{branch}</span>
          </div>
          <h2 className="mt-2 text-base font-semibold leading-tight">Diff ({entries.length})</h2>
          <p className="mt-1 text-[10px] text-muted-foreground">
            Read-only preview. Resolve conflicts via trogon-atlas or MCP; this view cannot write.
          </p>
        </div>
        <Button variant="ghost" size="icon" className="ml-auto" onClick={onClose}>
          <X />
        </Button>
      </div>
      <Separator />
      <div className="flex min-h-0 flex-1">
        <div className="w-44 shrink-0 space-y-3 overflow-y-auto border-r border-border p-3">
          {(['conflict', 'changed', 'added', 'deleted'] as const).map((status) =>
            grouped[status].length > 0 ? (
              <section key={status}>
                <h3 className="mb-1 flex items-center gap-1.5 text-[10px] font-semibold uppercase text-muted-foreground">
                  {STATUS_LABEL[status]}
                  <span className="rounded-full bg-muted px-1.5 text-[9px] font-normal">{grouped[status].length}</span>
                </h3>
                <ul className="space-y-1">
                  {grouped[status].map(({ entry, idx }) => (
                    <li key={entryKey(entry, idx)}>
                      <button
                        type="button"
                        onClick={() => setSelectedIdx(idx)}
                        className={cn(
                          'w-full truncate rounded-md border px-2 py-1 text-left font-mono text-[10px] hover:bg-accent',
                          selectedIdx === idx ? 'border-ring bg-accent' : 'border-border',
                        )}
                        title={entryLabel(entry)}
                      >
                        {entryLabel(entry)}
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            ) : null,
          )}
          {entries.length === 0 ? (
            <p className="text-[11px] text-muted-foreground">No changes on this branch.</p>
          ) : null}
        </div>
        <div className="min-w-0 flex-1 overflow-y-auto p-3">
          {!selected ? (
            <p className="text-[11px] text-muted-foreground">Select an entry to inspect it.</p>
          ) : (
            <div className="space-y-3">
              <div className="flex items-center gap-2">
                <span
                  className={cn(
                    'rounded border px-1.5 py-0.5 text-[10px] font-bold uppercase',
                    selectedStatus ? STATUS_BADGE_CLASS[selectedStatus] : 'border-border bg-muted',
                  )}
                >
                  {selectedStatus ? STATUS_LABEL[selectedStatus] : 'unknown'}
                </span>
                <span className="truncate font-mono text-[11px] text-muted-foreground">{entryLabel(selected)}</span>
              </div>
              {(selected.conflictFieldPaths ?? []).length > 0 ? (
                <div>
                  <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-red-700">
                    Conflicting fields
                  </div>
                  <ul className="flex flex-wrap gap-1">
                    {(selected.conflictFieldPaths ?? []).map((path) => (
                      <li
                        key={path}
                        className="rounded border border-red-300 bg-red-50 px-1.5 py-0.5 font-mono text-[10px] text-red-800"
                      >
                        {path}
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
              <div className="flex gap-3 overflow-x-auto">
                <JsonPane title="base" value={selected.base} />
                <JsonPane title="ours (branch)" value={selected.ours} />
                <JsonPane title="theirs (baseline)" value={selected.theirs} />
              </div>
            </div>
          )}
        </div>
      </div>
    </aside>
  );
}

import type { ScenarioExample } from '@/lib/model';

/** One scenario example: the entity it names and its example data, if any. */
export function ExampleBlock({ label, example }: { label?: string; example: ScenarioExample }) {
  const rows = Object.entries(example.data ?? {});
  return (
    <div className="rounded-md border border-border bg-muted/30 px-2 py-1.5">
      <div className="flex items-center gap-1.5 font-mono text-[10px]">
        {label ? <span className="uppercase text-muted-foreground">{label}</span> : null}
        <span className="font-semibold">{example.id.slug}</span>
      </div>
      {rows.length > 0 ? (
        <table className="mt-1 w-full text-[10px]">
          <tbody>
            {rows.map(([k, v]) => (
              <tr key={k} className="border-t border-border/60">
                <td className="py-0.5 pr-2 font-mono text-muted-foreground">{k}</td>
                <td className="break-all py-0.5 font-mono">{typeof v === 'string' ? v : JSON.stringify(v)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </div>
  );
}

import { useEffect, useMemo, useState } from 'react';
import { Button } from '@/components/ui/button';
import {
  api,
  type CompileTypeLibraryResult,
  type ProtoSourceFile,
  type TypeLibraryDiagnostic,
  type TypeLibraryMessage,
} from '@/lib/api';
import { readBranchFromUrl } from '@/lib/branch';
import type { Entity, EntityId } from '@/lib/model';

const TYPE_URL_PREFIX = 'type.googleapis.com/';

function sourceFilesOf(entity: Entity): ProtoSourceFile[] {
  const files = entity.raw.files;
  if (!Array.isArray(files)) return [];
  return files.flatMap((f) => {
    if (!f || typeof f !== 'object') return [];
    const { path, content } = f as Record<string, unknown>;
    return typeof path === 'string' ? [{ path, content: typeof content === 'string' ? content : '' }] : [];
  });
}

function isEntityId(v: unknown): v is EntityId {
  if (!v || typeof v !== 'object') return false;
  const { namespace, slug } = v as Record<string, unknown>;
  return typeof namespace === 'string' && typeof slug === 'string';
}

function dependenciesOf(entity: Entity): { id: EntityId }[] {
  const deps = entity.raw.dependencies;
  if (!Array.isArray(deps)) return [];
  return deps.flatMap((d) => {
    const id = (d as { id?: unknown } | null)?.id;
    return isEntityId(id)
      ? [{ id: { namespace: id.namespace, slug: id.slug, version: String(id.version ?? '0') } }]
      : [];
  });
}

export function diagnosticLocation(d: TypeLibraryDiagnostic): string {
  const parts = [d.path ?? ''];
  if (d.line) parts.push(String(d.line));
  if (d.line && d.column) parts.push(String(d.column));
  return parts.join(':');
}

type CompileState =
  | { status: 'idle' }
  | { status: 'compiling' }
  | { status: 'done'; result: CompileTypeLibraryResult }
  | { status: 'failed'; error: string };

/**
 * Edits a type library's proto sources locally and compiles them on the
 * server without writing anything; saving stays with agents and the CLI.
 */
export function TypeLibrarySources({ entity }: { entity: Entity }) {
  const original = useMemo(() => sourceFilesOf(entity), [entity]);
  const [files, setFiles] = useState(original);
  const [selected, setSelected] = useState(0);
  const [compile, setCompile] = useState<CompileState>({ status: 'idle' });

  useEffect(() => {
    setFiles(original);
    setSelected(0);
    setCompile({ status: 'idle' });
  }, [original]);

  const current = files[selected];
  const edited = files.some((f, i) => f.content !== original[i]?.content);

  async function runCompile() {
    setCompile({ status: 'compiling' });
    try {
      const result = await api.compileTypeLibrary(
        { id: entity.id, files, dependencies: dependenciesOf(entity) },
        { branch: readBranchFromUrl() },
      );
      setCompile({ status: 'done', result });
    } catch (e) {
      setCompile({ status: 'failed', error: e instanceof Error ? e.message : String(e) });
    }
  }

  if (files.length === 0) return null;
  return (
    <section>
      <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Sources</h3>
      <div className="flex flex-col gap-2">
        <select
          aria-label="Source file"
          className="rounded border border-border bg-card px-2 py-1 font-mono text-xs"
          value={selected}
          onChange={(e) => setSelected(Number(e.target.value))}
        >
          {files.map((f, i) => (
            <option key={f.path} value={i}>
              {f.path}
            </option>
          ))}
        </select>
        <textarea
          aria-label={`Source of ${current?.path ?? ''}`}
          spellCheck={false}
          className="h-64 w-full resize-y rounded border border-border bg-muted/40 p-2 font-mono text-[11px] leading-snug"
          value={current?.content ?? ''}
          onChange={(e) => {
            const content = e.target.value;
            setFiles((prev) => prev.map((f, i) => (i === selected ? { ...f, content } : f)));
          }}
        />
        <div className="flex items-center gap-2">
          <Button size="sm" variant="outline" onClick={runCompile} disabled={compile.status === 'compiling'}>
            {compile.status === 'compiling' ? 'Compiling' : 'Compile'}
          </Button>
          {edited ? (
            <span className="text-[11px] text-muted-foreground">
              Edits stay in this panel; an agent or the CLI saves them.
            </span>
          ) : null}
        </div>
        <CompileOutcome state={compile} />
      </div>
    </section>
  );
}

function CompileOutcome({ state }: { state: CompileState }) {
  if (state.status === 'failed') {
    return <p className="text-xs text-red-700">{state.error}</p>;
  }
  if (state.status !== 'done') return null;
  const diagnostics = state.result.diagnostics ?? [];
  const violations = state.result.compatibilityViolations ?? [];
  if (diagnostics.length === 0 && violations.length === 0) {
    return <p className="text-xs text-emerald-700">Compiles, and stays wire compatible.</p>;
  }
  return (
    <ul className="flex flex-col gap-1 text-xs">
      {diagnostics.map((d, i) => (
        <li key={`d-${i}-${diagnosticLocation(d)}`} className="text-red-700">
          <span className="font-mono">{diagnosticLocation(d)}</span> {d.message}
        </li>
      ))}
      {violations.map((v, i) => (
        <li key={`v-${i}-${v.message}`} className="text-amber-700">
          {v.message}
        </li>
      ))}
    </ul>
  );
}

function payloadTypeOf(entity: Entity): string | undefined {
  const schema = entity.raw.schema;
  if (!schema || typeof schema !== 'object') return undefined;
  const s = schema as Record<string, unknown>;
  const url = s['@type'] ?? s.typeUrl;
  return typeof url === 'string' ? url.split('/').pop() : undefined;
}

/**
 * Lists the messages the entity's namespace declares, so a payload type can
 * be chosen by name instead of typed by hand.
 */
export function PayloadTypePicker({ entity }: { entity: Entity }) {
  const [messages, setMessages] = useState<TypeLibraryMessage[] | undefined>();
  const [chosen, setChosen] = useState(payloadTypeOf(entity) ?? '');
  const namespace = entity.id.namespace;

  useEffect(() => {
    setChosen(payloadTypeOf(entity) ?? '');
  }, [entity]);

  useEffect(() => {
    const ctrl = new AbortController();
    api
      .typeMessages(namespace, { signal: ctrl.signal, branch: readBranchFromUrl() })
      .then((res) => setMessages(res.messages ?? []))
      .catch(() => {
        if (!ctrl.signal.aborted) setMessages([]);
      });
    return () => ctrl.abort();
  }, [namespace]);

  if (!messages || (messages.length === 0 && !chosen)) return null;
  const names = messages.map((m) => m.fullName);
  return (
    <section>
      <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Payload type</h3>
      <div className="flex flex-col gap-1">
        <select
          aria-label="Payload type"
          className="rounded border border-border bg-card px-2 py-1 font-mono text-xs"
          value={chosen}
          onChange={(e) => setChosen(e.target.value)}
        >
          {chosen && !names.includes(chosen) ? <option value={chosen}>{chosen}</option> : null}
          {chosen ? null : <option value="">Field list (built-in Schema)</option>}
          {names.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
        {chosen ? (
          <code className="break-all text-[11px] text-muted-foreground">{TYPE_URL_PREFIX + chosen}</code>
        ) : null}
      </div>
    </section>
  );
}

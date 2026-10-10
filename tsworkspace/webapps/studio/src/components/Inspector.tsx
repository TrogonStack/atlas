import { ExternalLink, FileCode2, Split, TriangleAlert, X } from 'lucide-react';
import { useMemo } from 'react';
import { CopyContextButton } from '@/components/CopyContextButton';
import { CopyFixPromptButton } from '@/components/CopyFixPromptButton';
import { kindStyle } from '@/components/canvas/StickyNode';
import { PayloadTypePicker, TypeLibrarySources } from '@/components/TypeLibraryPanels';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { readBranchFromUrl, withBranchQuery } from '@/lib/branch';
import { entityContext } from '@/lib/context';
import { EMPTY_ISSUE_INDEX, type Issue, type IssueIndex, type Severity } from '@/lib/issues';
import {
  type Annotation,
  ambiguityRulingOf,
  ambiguityTermsOf,
  asId,
  classificationOf,
  contextSeams,
  domainOf,
  type Entity,
  fieldTypeLabel,
  humanStatus,
  type Model,
  type NeighborView,
  neighborsOf,
  neighborsOfMoment,
  orphanOf,
  realizedBy,
  realizesOf,
  relationshipsOf,
  type ScenarioExample,
  type ScenarioView,
  screenOfUi,
  screenSlotsOf,
  staleRefsOf,
  storyboardEntryReadModel,
  subdomainsOf,
  termEmbodiedBy,
  trackingOf,
  uiTransitionsOf,
} from '@/lib/model';
import { useEntityHomes } from '@/lib/useEntityHomes';
import { isSafeNamespace } from '../../shared/safe-namespace.mjs';

const ALLOWED_PROTOCOLS = new Set(['https:', 'http:', 'mailto:']);

/**
 * Returns the href only when the URL uses a safe protocol.
 * Rejects javascript:, data:, and any other potentially executable scheme.
 */
export function safeLinkHref(raw: string): string {
  if (!raw || raw === '#') return '#';
  try {
    const parsed = new URL(raw);
    return ALLOWED_PROTOCOLS.has(parsed.protocol) ? raw : '#';
  } catch {
    // Relative URLs have no protocol and are safe to pass through.
    return raw.startsWith('/') || raw.startsWith('.') || raw.startsWith('#') ? raw : '#';
  }
}

// Cross-context seams: subscription views point at upstream events
// from other namespaces, and events may be subscribed by views in
// other namespaces. When the upstream entity isn't loaded in the
// current model (typical for an EM-scoped page), surface jump links
// to whichever EventModels claim the upstream/downstream entity as a
// member; clicking opens that EM with the entity focused.
function SeamSection({ entity, model, onSelect }: { entity: Entity; model: Model; onSelect: (key: string) => void }) {
  const upstreamRefs =
    entity.kind === 'readModel'
      ? (Array.isArray(entity.raw.sourceEvents) ? entity.raw.sourceEvents : [])
          .map((r) => asId((r as Record<string, unknown>)?.id))
          .filter((id) => id.namespace && id.slug && id.namespace !== entity.id.namespace)
      : [];
  const allSeams = useMemo(() => contextSeams(model), [model]);
  const downstreamViews =
    entity.kind === 'event'
      ? allSeams.filter(
          (s) => s.upstreamEvent.namespace === entity.id.namespace && s.upstreamEvent.slug === entity.id.slug,
        )
      : [];
  // Memoize so the refs array identity is stable across renders: the
  // hook's useMemo would otherwise rerun on every render if refs were
  // rebuilt inline each time.
  const entityRefs = useMemo(() => upstreamRefs.map((id) => ({ kind: 'event', id })), [upstreamRefs]);
  // Fetch home EMs for every cross-context ref so unloaded upstreams
  // get an "Open in <em>" link instead of a dead-end "not loaded" row.
  const { homes, error: homesError, retry: retryHomes } = useEntityHomes(entityRefs);
  if (upstreamRefs.length === 0 && downstreamViews.length === 0) return null;
  return (
    <section>
      <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Context seams</h3>
      {homesError ? (
        <div className="mb-1.5 flex items-center gap-2 rounded-md border border-red-300 bg-red-50 px-2 py-1 text-xs text-red-700">
          <span>Could not load event model index.</span>
          <button type="button" onClick={retryHomes} className="underline">
            Retry
          </button>
        </div>
      ) : null}
      <div className="space-y-1.5">
        {upstreamRefs.map((id) => {
          const label = `${id.namespace}/${id.slug}`;
          const ems = homes.get(label) ?? [];
          const focusParam = encodeURIComponent(`event:${label}`);
          return (
            <div key={label} className="rounded-md border border-fuchsia-300 bg-fuchsia-50 px-2 py-1.5 text-xs">
              <div>
                translated from <span className="font-mono font-semibold">{label}</span>
              </div>
              {ems.length > 0 ? (
                <div className="mt-2 space-y-1">
                  {ems.map((em) => {
                    const url = `/em/${em.id.namespace}/${em.id.slug}?focus=${focusParam}`;
                    return (
                      <button
                        key={em.key}
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          if (!isSafeNamespace(em.id.namespace) || !isSafeNamespace(em.id.slug)) {
                            console.warn('Inspector: skipping navigation, invalid namespace or slug', em.id);
                            return;
                          }
                          // Hard navigation: studio router re-parses
                          // on a full load. Always navigate to the
                          // upstream EM (even when the event is in
                          // model.entities as a halo): the halo entity
                          // has no rendered sticky on the board after
                          // the halo-only-band drop, so an in-page
                          // select would do nothing visible.
                          window.location.href = withBranchQuery(url, readBranchFromUrl());
                        }}
                        title={`Navigate to ${url}`}
                        className="flex w-full items-center justify-between gap-2 rounded-md border border-fuchsia-400 bg-fuchsia-100 px-2 py-1.5 text-left text-xs font-medium text-fuchsia-800 hover:bg-fuchsia-200"
                      >
                        <span className="truncate">↗ Open in {em.title || `${em.id.namespace}/${em.id.slug}`}</span>
                        <span className="shrink-0 font-mono text-[9px] text-fuchsia-600">{em.id.namespace}</span>
                      </button>
                    );
                  })}
                </div>
              ) : (
                <span className="mt-1 block text-[10px] text-muted-foreground">
                  no event model curates this upstream; context not loaded
                </span>
              )}
            </div>
          );
        })}
        {downstreamViews.map((s) => (
          <button
            key={s.view.key}
            type="button"
            onClick={() => onSelect(s.view.key)}
            className="w-full rounded-md border border-fuchsia-300 bg-fuchsia-50 px-2 py-1.5 text-left text-xs hover:bg-fuchsia-100"
          >
            feeds context <span className="font-mono font-semibold">{s.view.id.namespace}</span>: {s.view.title}
          </button>
        ))}
      </div>
      <p className="mt-1 text-[10px] text-muted-foreground">
        Contexts touch only here: published events flowing into subscription views.
      </p>
    </section>
  );
}

// Timeline navigation: clicking a neighbor selects it, highlights it, and
// pans the canvas to it in whichever view is active.
function NeighborList({
  title,
  arrow,
  items,
  onSelect,
}: {
  title: string;
  arrow: 'back' | 'fwd';
  items: NeighborView[];
  onSelect: (key: string) => void;
}) {
  if (items.length === 0) return null;
  return (
    <div>
      <div className="mb-1 text-[10px] font-bold uppercase tracking-wide text-muted-foreground">{title}</div>
      <div className="space-y-1">
        {items.map((n) => {
          const style = kindStyle(n.entity.kind);
          const Icon = style.icon;
          return (
            <button
              key={n.entity.key}
              type="button"
              onClick={() => onSelect(n.entity.key)}
              className="flex w-full items-center gap-1.5 rounded-md border border-border px-2 py-1.5 text-left text-xs hover:bg-accent"
            >
              {arrow === 'back' ? <span className="shrink-0 text-muted-foreground">◀</span> : null}
              <Icon className="h-3 w-3 shrink-0 text-muted-foreground" />
              <span className="truncate font-medium">{n.entity.title}</span>
              <span className="ml-auto shrink-0 truncate text-[9px] text-muted-foreground">{n.relation}</span>
              {arrow === 'fwd' ? <span className="shrink-0 text-muted-foreground">▶</span> : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function ExampleBlock({ label, example }: { label?: string; example: ScenarioExample }) {
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

function ScenarioBlock({ scenario, givenApplies }: { scenario: ScenarioView; givenApplies: boolean }) {
  const outcome = scenario.reject
    ? `✗ ${scenario.reject.reasonCode}`
    : scenario.thenEmits.length > 0
      ? `→ ${scenario.thenEmits.map((e) => e.id.slug).join(', ')}`
      : scenario.thenState
        ? `state: ${scenario.thenState.id.slug}`
        : scenario.thenCommand
          ? `→ ${scenario.thenCommand.id.slug}`
          : '';
  return (
    <details className="rounded-md border border-border">
      <summary className="cursor-pointer px-2 py-1.5 text-xs">
        <span className="font-medium">{scenario.title || scenario.id}</span>
        <span className={`ml-2 font-mono text-[10px] ${scenario.reject ? 'text-red-600' : 'text-muted-foreground'}`}>
          {outcome}
        </span>
      </summary>
      <div className="space-y-2 border-t border-border px-2 py-2">
        {scenario.doc ? <p className="text-[11px] leading-snug text-muted-foreground">{scenario.doc}</p> : null}
        {/* Shown even when empty: "given nothing" says this moment starts from
            no prior state, which is a design claim, not a missing section.
            Read-model slices have no precondition to state: their inputs are
            the WHEN, so the section stays off for them entirely. */}
        {givenApplies ? (
          <div className="space-y-1">
            <div className="text-[10px] font-bold uppercase tracking-wide text-muted-foreground">Given</div>
            {scenario.given.length > 0 ? (
              scenario.given.map((g, i) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: examples have no stable id
                <ExampleBlock key={i} example={g} />
              ))
            ) : (
              <p className="text-[11px] italic text-muted-foreground">nothing</p>
            )}
          </div>
        ) : null}
        {scenario.when.length > 0 || scenario.whenDoc ? (
          <div className="space-y-1">
            <div className="text-[10px] font-bold uppercase tracking-wide text-muted-foreground">When</div>
            {scenario.whenDoc ? <p className="text-[11px] italic">{scenario.whenDoc}</p> : null}
            {scenario.when.map((w, i) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: examples have no stable id
              <ExampleBlock key={i} example={w} />
            ))}
          </div>
        ) : null}
        <div className="space-y-1">
          <div className="text-[10px] font-bold uppercase tracking-wide text-muted-foreground">Then</div>
          {scenario.reject ? (
            <div className="rounded-md border border-red-200 bg-red-50 px-2 py-1.5 text-[11px]">
              <span className="font-mono font-semibold text-red-700">{scenario.reject.reasonCode}</span>
              {scenario.reject.doc ? <p className="mt-0.5 text-red-800">{scenario.reject.doc}</p> : null}
            </div>
          ) : null}
          {scenario.thenEmits.map((e, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: examples have no stable id
            <ExampleBlock key={i} example={e} />
          ))}
          {scenario.thenState ? <ExampleBlock example={scenario.thenState} /> : null}
          {scenario.thenCommand ? <ExampleBlock example={scenario.thenCommand} /> : null}
        </div>
      </div>
    </details>
  );
}

// "SharedConsumersAnnotation" -> "Shared consumers": the proto message name is
// the only label an unmapped annotation type has.
function annotationLabel(type: string): string {
  const words = type
    .replace(/Annotation$/, '')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function AnnotationView({ annotation }: { annotation: Annotation }) {
  const d = annotation.data;
  switch (annotation.type) {
    case 'Uuidv5IdentityAnnotation': {
      const ns = d.dns ?? d.url ?? d.uuid;
      const nsKind = d.dns ? 'dns' : d.url ? 'url' : 'uuid';
      return (
        <div className="rounded-md border border-border px-2 py-1.5 text-xs">
          <div className="flex items-center gap-1.5">
            <span className="font-semibold">UUIDv5 identity</span>
            {d.name ? <span className="rounded bg-muted px-1.5 font-mono text-[10px]">{String(d.name)}</span> : null}
            {d.version !== undefined ? <Badge variant="outline">v{String(d.version)}</Badge> : null}
          </div>
          <p className="mt-1 font-mono text-[10px]">{String(d.template ?? '')}</p>
          {ns ? (
            <p className="mt-0.5 text-[10px] text-muted-foreground">
              namespace ({nsKind}): <span className="font-mono">{String(ns)}</span>
            </p>
          ) : null}
        </div>
      );
    }
    case 'CodeRefAnnotation': {
      const repo = String(d.repo ?? '');
      const path = String(d.path ?? '');
      const symbol = String(d.symbol ?? '');
      const ref = String(d.ref ?? '');
      const cut = path.lastIndexOf('/');
      const dir = cut >= 0 ? path.slice(0, cut + 1) : '';
      const file = cut >= 0 ? path.slice(cut + 1) : path;
      return (
        <div className="rounded-md border border-border px-2 py-1.5 text-xs">
          <div className="flex items-center gap-1.5">
            <FileCode2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <span className="font-semibold">Implemented in</span>
            {repo ? <Badge variant="outline">{repo}</Badge> : null}
            {ref ? (
              <Badge variant="secondary" className="ml-auto font-mono" title={ref}>
                {ref.slice(0, 7)}
              </Badge>
            ) : null}
          </div>
          {path ? (
            <p className="mt-1 break-all font-mono text-[10px] leading-snug">
              <span className="text-muted-foreground">{dir}</span>
              <span className="font-semibold">{file}</span>
            </p>
          ) : null}
          {symbol ? <p className="mt-0.5 break-all font-mono text-[10px] text-muted-foreground">{symbol}</p> : null}
          {!repo || !path ? (
            <p className="mt-1 text-[10px] text-amber-700">
              CODE_REF_INCOMPLETE: a code reference needs both a logical repo name and a repo-relative path to be
              resolvable.
            </p>
          ) : null}
        </div>
      );
    }
    case 'LinkAnnotation':
      return (
        <a
          href={safeLinkHref(String(d.url ?? '#'))}
          target="_blank"
          rel="noreferrer"
          className="flex items-center gap-1.5 rounded-md border border-border px-2 py-1.5 text-xs hover:bg-accent"
        >
          <ExternalLink className="h-3.5 w-3.5 text-muted-foreground" />
          <span className="truncate">{String(d.title || d.url || '')}</span>
          {d.rel ? (
            <Badge variant="outline" className="ml-auto">
              {String(d.rel)}
            </Badge>
          ) : null}
        </a>
      );
    case 'TagAnnotation': {
      const tags = Array.isArray(d.tags) ? d.tags : [];
      return (
        <div className="flex flex-wrap gap-1">
          {tags.map((t) => (
            <Badge key={String(t)} variant="outline">
              {String(t)}
            </Badge>
          ))}
        </div>
      );
    }
    case 'LifecycleAnnotation':
      return (
        <div className="rounded-md border border-border px-2 py-1.5 text-xs">
          <Badge variant="secondary">{String(d.status ?? '')}</Badge>
          {d.doc ? <span className="ml-2 text-muted-foreground">{String(d.doc)}</span> : null}
        </div>
      );
    case 'FailureModeAnnotation':
      return (
        <div className="rounded-md border border-border px-2 py-1.5 text-xs">
          <div className="flex items-center gap-1.5">
            <TriangleAlert className="h-3.5 w-3.5 shrink-0 text-amber-700" />
            <span className="font-semibold">{String(d.trigger ?? '')}</span>
          </div>
          <p className="mt-1 leading-snug">{String(d.behavior ?? '')}</p>
          {d.recovery ? <p className="mt-0.5 text-[10px] text-muted-foreground">{String(d.recovery)}</p> : null}
        </div>
      );
    case 'NoteAnnotation':
    case 'OpenQuestionAnnotation':
      return (
        <div className="rounded-md border border-border px-2 py-1.5 text-xs">
          <p className="leading-snug">{String(d.text ?? d.question ?? '')}</p>
          {d.author ? <p className="mt-0.5 text-[10px] text-muted-foreground">({String(d.author)})</p> : null}
        </div>
      );
    default: {
      const entries = Object.entries(d).filter(([, v]) => v !== null && v !== undefined && v !== '');
      return (
        <div className="rounded-md border border-border px-2 py-1.5 text-xs">
          <div className="font-semibold">{annotationLabel(annotation.type)}</div>
          {entries.length > 0 ? (
            <dl className="mt-1 space-y-0.5">
              {entries.map(([k, v]) => (
                <div key={k} className="flex gap-1.5">
                  <dt className="shrink-0 text-[10px] uppercase tracking-wide text-muted-foreground">
                    {k.replace(/_/g, ' ')}
                  </dt>
                  <dd className="min-w-0 break-words text-[10px]">
                    {typeof v === 'object' ? JSON.stringify(v) : String(v)}
                  </dd>
                </div>
              ))}
            </dl>
          ) : null}
        </div>
      );
    }
  }
}

const SEVERITY_STYLE: Record<Severity, string> = {
  error: 'border-red-300 bg-red-50 text-red-800',
  warning: 'border-amber-300 bg-amber-50 text-amber-900',
  info: 'border-sky-200 bg-sky-50 text-sky-800',
};

// The validator's findings for this entity, rendered above everything else
// the panel claims about the model. A silent panel and a clean panel used
// to look identical, so an entity the server rejects read as healthy.
// `error` is not swallowed: "validation did not run" and "validation found
// nothing" are different states and must not render the same.
function IssueList({
  issues,
  error,
  model,
  momentKey,
}: {
  issues: Issue[];
  error?: string;
  model: Model;
  momentKey?: string;
}) {
  if (error)
    return (
      <section>
        <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Validation</h3>
        <p className="text-[11px] italic text-muted-foreground">
          unavailable ({error}). This panel cannot say whether the entity is valid.
        </p>
      </section>
    );
  if (issues.length === 0) return null;
  return (
    <section>
      <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Validation ({issues.length})</h3>
      <div className="flex flex-col gap-2">
        {issues.map((i) => (
          <div
            key={`${i.code} ${i.field} ${i.message}`}
            className={`flex flex-col gap-1.5 rounded border px-2 py-1.5 ${SEVERITY_STYLE[i.severity]}`}
          >
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="text-[9px] font-bold uppercase tracking-wide">{i.severity}</span>
              <span className="font-mono text-[10px]">{i.code}</span>
              {i.field ? <span className="font-mono text-[10px] opacity-70">{i.field}</span> : null}
            </div>
            {i.ruleTitle ? <div className="text-[11px] font-semibold">{i.ruleTitle}</div> : null}
            <p className="text-[11px] leading-snug">{i.message}</p>
            {i.defaultSeverity && i.defaultSeverity !== i.severity ? (
              <p className="text-[10px] italic opacity-70">rule default: {i.defaultSeverity}</p>
            ) : null}
            <CopyFixPromptButton model={model} issue={i} momentKey={momentKey} />
          </div>
        ))}
      </div>
    </section>
  );
}

export function Inspector({
  model,
  entity,
  instanceUid,
  momentKey,
  issues = EMPTY_ISSUE_INDEX,
  onClose,
  onSelect,
}: {
  model: Model;
  entity: Entity;
  /** Validator findings for the whole model; this panel reads only its own. */
  issues?: IssueIndex;
  // The clicked moment's machine id: uuidv5(type uid / defining-slice uid).
  // Present only for canvas occurrence selections; distinct per instance.
  instanceUid?: string;
  // The clicked moment's defining slice: makes the Timeline instance-aware.
  momentKey?: string;
  onClose: () => void;
  onSelect: (key: string) => void;
}) {
  const style = kindStyle(entity.kind);
  const ownIssues = useMemo(() => issues.for(entity), [issues, entity]);
  const ownSlice = useMemo(() => model.slices.find((s) => s.entity.key === entity.key), [model, entity]);
  const sliceScenarios = ownSlice?.scenarios ?? [];
  const memberships = useMemo(
    () =>
      model.slices.filter((s) => {
        const refs = [
          s.persona && `persona:${s.persona.id.namespace}/${s.persona.id.slug}`,
          s.ui && `ui:${s.ui.id.namespace}/${s.ui.id.slug}`,
          s.command && `command:${s.command.id.namespace}/${s.command.id.slug}`,
          s.readModel && `readModel:${s.readModel.id.namespace}/${s.readModel.id.slug}`,
          s.processor && `processor:${s.processor.id.namespace}/${s.processor.id.slug}`,
          ...s.events.map((e) => `event:${e.id.namespace}/${e.id.slug}`),
          ...s.readModels.map((e) => `readModel:${e.id.namespace}/${e.id.slug}`),
        ].filter(Boolean);
        return refs.includes(`${entity.kind}:${entity.id.namespace}/${entity.id.slug}`);
      }),
    [model, entity],
  );

  const { backward: neighborsBackward, forward: neighborsForward } = useMemo(
    () => (momentKey ? neighborsOfMoment(model, entity, momentKey) : neighborsOf(model, entity)),
    [model, entity, momentKey],
  );

  // OrphanAnnotation already has its own section above; listing it again here
  // would show the same doc twice.
  const listedAnnotations = useMemo(
    () => entity.annotations.filter((a) => a.type !== 'OrphanAnnotation'),
    [entity.annotations],
  );

  return (
    <aside className="flex h-full w-80 flex-col border-l border-border bg-card">
      <div className="flex items-start gap-2 p-4">
        <div>
          <div className="flex items-center gap-2">
            <Badge variant="secondary">{style.label}</Badge>
            {classificationOf(entity) ? (
              <span
                title="core/supporting/generic: where investment goes (Decision #29)"
                className={`rounded px-1.5 py-0.5 text-[10px] font-bold uppercase ${
                  classificationOf(entity) === 'core'
                    ? 'bg-amber-100 text-amber-800'
                    : classificationOf(entity) === 'generic'
                      ? 'bg-zinc-100 text-zinc-600'
                      : 'bg-sky-100 text-sky-700'
                }`}
              >
                {classificationOf(entity)}
              </span>
            ) : null}
            <span className="font-mono text-[11px] text-muted-foreground">v{entity.id.version}</span>
          </div>
          <h2 className="mt-2 text-base font-semibold leading-tight">{entity.title}</h2>
          <div className="font-mono text-[11px] text-muted-foreground">
            {entity.id.namespace}/{entity.id.slug}
          </div>
          {instanceUid ? (
            <div
              title="the defining slice's uid scoped by the type's uid (uuidv5): this moment, not the type"
              className="mt-1 flex items-center gap-1.5"
            >
              <span className="rounded border border-border bg-muted px-1 text-[8px] font-semibold uppercase tracking-wide text-muted-foreground">
                id
              </span>
              <span className="font-mono text-[10px] text-muted-foreground">{instanceUid}</span>
            </div>
          ) : null}
        </div>
        <Button variant="ghost" size="icon" className="ml-auto" onClick={onClose}>
          <X />
        </Button>
      </div>
      <div className="px-4 pb-3">
        <CopyContextButton getText={() => entityContext(model, entity, momentKey, ownIssues)} />
      </div>
      <Separator />
      <div className="flex-1 space-y-4 overflow-y-auto p-4 text-sm">
        <IssueList issues={ownIssues} error={issues.error} model={model} momentKey={momentKey} />
        {neighborsBackward.length > 0 || neighborsForward.length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">
              Timeline{momentKey ? ': this moment' : ''}
            </h3>
            <div className="space-y-2">
              <NeighborList title="Backward" arrow="back" items={neighborsBackward} onSelect={onSelect} />
              <NeighborList title="Forward" arrow="fwd" items={neighborsForward} onSelect={onSelect} />
            </div>
          </section>
        ) : null}
        {entity.doc ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Doc</h3>
            <p className="leading-relaxed text-foreground/90">{entity.doc}</p>
          </section>
        ) : null}
        {entity.role ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Role</h3>
            <Badge variant="outline">{entity.role}</Badge>
          </section>
        ) : null}
        {entity.streamId ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Stream identity</h3>
            <p className="text-xs leading-relaxed">
              <span className="rounded bg-muted px-1.5 py-0.5 font-mono">{entity.streamId}</span>
            </p>
          </section>
        ) : null}
        {entity.externalSource ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">External source</h3>
            <p className="text-xs leading-relaxed text-foreground/90">
              Populated by <span className="font-semibold">{entity.externalSource.system}</span>
              {entity.externalSource.descriptor ? ` (${entity.externalSource.descriptor})` : ''}: not projected from
              internal events.
            </p>
          </section>
        ) : null}
        {entity.kind === 'typeLibrary' ? <TypeLibrarySources entity={entity} /> : null}
        {entity.kind === 'event' || entity.kind === 'command' || entity.kind === 'readModel' ? (
          <PayloadTypePicker entity={entity} />
        ) : null}
        <SeamSection entity={entity} model={model} onSelect={onSelect} />
        {(() => {
          const find = (kind: Entity['kind'], id: { namespace: string; slug: string }) =>
            model.entities.find((e) => e.kind === kind && e.id.namespace === id.namespace && e.id.slug === id.slug);
          const rows: React.ReactNode[] = [];
          if (entity.kind === 'ui' && entity.screenSlot) {
            const screen = screenOfUi(model, entity);
            rows.push(
              screen ? (
                <button
                  key="screen"
                  type="button"
                  onClick={() => onSelect(screen.key)}
                  className="w-full rounded-md border border-teal-200 bg-teal-50/70 px-2 py-1.5 text-left text-xs hover:bg-teal-100"
                >
                  fills <span className="font-mono font-semibold">{entity.screenSlot.slot}</span> on{' '}
                  <span className="font-mono font-semibold">{screen.title}</span>
                </button>
              ) : (
                <div key="screen" className="rounded-md border border-teal-200 bg-teal-50/70 px-2 py-1.5 text-xs">
                  fills <span className="font-mono font-semibold">{entity.screenSlot.slot}</span> on{' '}
                  <span className="font-mono font-semibold">
                    {entity.screenSlot.screen.namespace}/{entity.screenSlot.screen.slug}
                  </span>
                </div>
              ),
            );
          } else if (entity.kind === 'screen') {
            for (const slot of screenSlotsOf(model, entity)) {
              rows.push(
                <div key={slot.name} className="rounded-md border border-teal-200 bg-teal-50/70 px-2 py-1.5 text-xs">
                  <div className="font-mono font-semibold">
                    {slot.name}
                    {slot.required ? <span className="ml-1 text-[10px] uppercase text-teal-700">required</span> : null}
                  </div>
                  {slot.doc ? <div className="text-[10px] text-muted-foreground">{slot.doc}</div> : null}
                  <div className="mt-1 space-y-1">
                    {slot.contributors.length > 0 ? (
                      slot.contributors.map((ui) => (
                        <button
                          key={ui.key}
                          type="button"
                          onClick={() => onSelect(ui.key)}
                          className="block w-full rounded border border-teal-200 bg-white/60 px-1.5 py-1 text-left font-mono text-[10px] hover:bg-teal-100"
                        >
                          {ui.id.namespace}/{ui.id.slug}
                        </button>
                      ))
                    ) : (
                      <span className="text-[10px] text-muted-foreground">unfilled</span>
                    )}
                  </div>
                </div>,
              );
            }
          } else if (entity.kind === 'term') {
            for (const ref of termEmbodiedBy(entity)) {
              const target = find(ref.kind, ref.id);
              rows.push(
                target ? (
                  <button
                    key={`${ref.kind}:${ref.id.namespace}/${ref.id.slug}`}
                    type="button"
                    onClick={() => onSelect(target.key)}
                    className="w-full rounded-md border border-lime-200 bg-lime-50/70 px-2 py-1.5 text-left text-xs hover:bg-lime-100"
                  >
                    embodied by <span className="font-mono font-semibold">{target.title}</span>
                  </button>
                ) : (
                  <div
                    key={`${ref.kind}:${ref.id.namespace}/${ref.id.slug}`}
                    className="rounded-md border border-lime-200 bg-lime-50/70 px-2 py-1.5 text-xs"
                  >
                    embodied by{' '}
                    <span className="font-mono font-semibold">
                      {ref.kind}:{ref.id.namespace}/{ref.id.slug}
                    </span>
                  </div>
                ),
              );
            }
          } else if (entity.kind === 'ambiguity') {
            const ruling = ambiguityRulingOf(entity);
            if (ruling) {
              rows.push(
                <div key="ruling" className="rounded-md border border-rose-200 bg-rose-50/70 px-2 py-1.5 text-xs">
                  <span className="font-semibold uppercase text-rose-700">{ruling}</span>
                </div>,
              );
            }
            for (const id of ambiguityTermsOf(entity)) {
              const term = find('term', id);
              rows.push(
                term ? (
                  <button
                    key={`${id.namespace}/${id.slug}`}
                    type="button"
                    onClick={() => onSelect(term.key)}
                    className="w-full rounded-md border border-rose-200 bg-rose-50/70 px-2 py-1.5 text-left text-xs hover:bg-rose-100"
                  >
                    term <span className="font-mono font-semibold">{term.title}</span>
                  </button>
                ) : (
                  <div
                    key={`${id.namespace}/${id.slug}`}
                    className="rounded-md border border-rose-200 bg-rose-50/70 px-2 py-1.5 text-xs"
                  >
                    term{' '}
                    <span className="font-mono font-semibold">
                      {id.namespace}/{id.slug}
                    </span>
                  </div>
                ),
              );
            }
          }
          if (rows.length === 0) return null;
          return (
            <section>
              <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">
                {entity.kind === 'screen' || entity.kind === 'ui' ? 'Screen composition' : 'Language'}
              </h3>
              <div className="space-y-1.5">{rows}</div>
            </section>
          );
        })()}
        {(() => {
          const stale = staleRefsOf(model, entity);
          if (stale.length === 0) return null;
          return (
            <section>
              <h3 className="mb-1 text-xs font-semibold uppercase text-amber-700">Migration pending</h3>
              <div className="space-y-1.5">
                {stale.map((sr) => (
                  <div
                    key={`${sr.label}:${sr.ref.namespace}/${sr.ref.slug}`}
                    className="rounded-md border border-amber-300 bg-amber-50 px-2 py-1.5 text-xs"
                  >
                    {sr.label} <span className="font-mono font-semibold">{sr.ref.slug}</span> pins{' '}
                    <span className="font-mono">v{sr.ref.version}</span>;{' '}
                    <span className="font-mono">v{sr.latest}</span> exists
                  </div>
                ))}
              </div>
              <p className="mt-1 text-[10px] text-muted-foreground">
                A breaking change is mid-flight: re-point the ref or keep both versions deliberately.
              </p>
            </section>
          );
        })()}
        {(() => {
          // Decision #29: the knowledge graph in the drawer realizes on
          // contexts, domain + realized-by on subdomains, subdomains on
          // domains. Targets may live in an unloaded namespace.
          const linkBtn = (e: Entity, prefix: string) => (
            <button
              key={`${prefix}:${e.key}`}
              type="button"
              onClick={() => onSelect(e.key)}
              className="w-full rounded-md border border-indigo-200 bg-indigo-50/60 px-2 py-1.5 text-left text-xs hover:bg-indigo-100"
            >
              {prefix} <span className="font-mono font-semibold">{e.title}</span>
              {classificationOf(e) ? (
                <span className="ml-1.5 text-[10px] font-bold uppercase text-indigo-700">{classificationOf(e)}</span>
              ) : null}
            </button>
          );
          const missing = (label: string, key: string) => (
            <div key={key} className="rounded-md border border-indigo-200 bg-indigo-50/60 px-2 py-1.5 text-xs">
              <span className="font-mono font-semibold">{label}</span>
              <span className="block text-[10px] text-muted-foreground">
                not loaded; switch to all namespaces to navigate
              </span>
            </div>
          );
          const find = (kind: Entity['kind'], id: { namespace: string; slug: string }) =>
            model.entities.find((e) => e.kind === kind && e.id.namespace === id.namespace && e.id.slug === id.slug);
          const rows: React.ReactNode[] = [];
          if (entity.kind === 'boundedContext') {
            for (const rid of realizesOf(entity)) {
              const sd = find('subdomain', rid);
              rows.push(sd ? linkBtn(sd, 'realizes') : missing(`${rid.namespace}/${rid.slug}`, `r:${rid.slug}`));
            }
            for (const rel of relationshipsOf(entity)) {
              const up = find('boundedContext', rel.upstream);
              rows.push(
                <div
                  key={`rel:${rel.upstream.namespace}`}
                  className="rounded-md border border-indigo-200 bg-indigo-50/60 px-2 py-1.5 text-xs"
                >
                  {up ? (
                    <button
                      type="button"
                      onClick={() => onSelect(up.key)}
                      className="font-mono font-semibold hover:underline"
                    >
                      {rel.upstream.namespace}
                    </button>
                  ) : (
                    <span className="font-mono font-semibold">{rel.upstream.namespace}</span>
                  )}
                  <span className="ml-1.5 rounded bg-indigo-100 px-1 text-[10px] font-bold uppercase text-indigo-700">
                    {rel.intent}
                  </span>
                  {rel.doc ? <span className="block text-[10px] text-muted-foreground">{rel.doc}</span> : null}
                </div>,
              );
            }
          } else if (entity.kind === 'subdomain') {
            const did = domainOf(entity);
            if (did) {
              const d = find('domain', did);
              rows.push(d ? linkBtn(d, 'belongs to') : missing(`${did.namespace}/${did.slug}`, 'domain'));
            }
            for (const bc of realizedBy(model, entity)) rows.push(linkBtn(bc, 'realized by'));
          } else if (entity.kind === 'domain') {
            for (const sd of subdomainsOf(model, entity)) rows.push(linkBtn(sd, 'contains'));
          }
          if (rows.length === 0) return null;
          return (
            <section>
              <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Problem space</h3>
              <div className="space-y-1.5">{rows}</div>
              <p className="mt-1 text-[10px] text-muted-foreground">
                Knowledge, not coupling: realizes maps solution to problem; no data flows.
              </p>
            </section>
          );
        })()}
        {uiTransitionsOf(entity).length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Navigates to</h3>
            <div className="space-y-1.5">
              {uiTransitionsOf(entity).map((t) => {
                const target = model.entities.find(
                  (e) => e.kind === 'ui' && e.id.namespace === t.to.namespace && e.id.slug === t.to.slug,
                );
                const label = `${t.to.namespace}/${t.to.slug}`;
                return target ? (
                  <button
                    key={label}
                    type="button"
                    onClick={() => onSelect(target.key)}
                    className="w-full rounded-md border border-border bg-muted/40 px-2 py-1.5 text-left text-xs hover:bg-accent"
                  >
                    <span className="font-mono font-semibold">{target.title}</span>
                    {t.doc ? <span className="block text-[10px] text-muted-foreground">{t.doc}</span> : null}
                  </button>
                ) : (
                  <div key={label} className="rounded-md border border-border bg-muted/40 px-2 py-1.5 text-xs">
                    <span className="font-mono font-semibold">{label}</span>
                    {t.doc ? <span className="block text-[10px] text-muted-foreground">{t.doc}</span> : null}
                  </div>
                );
              })}
            </div>
            <p className="mt-1 text-[10px] text-muted-foreground">
              Declared pure navigation: links no command path explains.
            </p>
          </section>
        ) : null}
        {trackingOf(model, entity).length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Tracking</h3>
            <div className="space-y-1.5">
              {trackingOf(model, entity).map((t) => (
                <button
                  key={t.tracker.key}
                  type="button"
                  onClick={() => onSelect(t.tracker.key)}
                  className="flex w-full items-baseline gap-2 rounded-md border border-border px-2 py-1.5 text-left text-xs hover:bg-accent"
                >
                  <span
                    className={
                      t.status === 'blocked'
                        ? 'rounded bg-red-100 px-1 font-mono text-[10px] font-bold text-red-700'
                        : t.status === 'done'
                          ? 'rounded bg-emerald-100 px-1 font-mono text-[10px] text-emerald-800'
                          : 'rounded bg-muted px-1 font-mono text-[10px]'
                    }
                  >
                    {t.status || 'todo'}
                  </span>
                  <span className="truncate text-muted-foreground">{t.tracker.title}</span>
                  {t.doc ? <span className="ml-auto truncate text-[10px] text-muted-foreground">{t.doc}</span> : null}
                </button>
              ))}
            </div>
          </section>
        ) : null}
        {entity.kind === 'tracker' && Array.isArray(entity.raw.items) && entity.raw.items.length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">
              Items ({entity.raw.items.length})
            </h3>
            <div className="overflow-hidden rounded-md border border-border">
              {(entity.raw.items as Record<string, unknown>[]).map((item, i) => {
                const subject = item.subject as { id?: { slug?: string } } | undefined;
                const status = humanStatus(item.status);
                return (
                  <div
                    // biome-ignore lint/suspicious/noArrayIndexKey: items have no stable id
                    key={i}
                    className="flex items-baseline gap-2 border-b border-border px-2 py-1.5 last:border-0"
                  >
                    <span
                      className={
                        status === 'blocked'
                          ? 'rounded bg-red-100 px-1 font-mono text-[10px] font-bold text-red-700'
                          : status === 'done'
                            ? 'rounded bg-emerald-100 px-1 font-mono text-[10px] text-emerald-800'
                            : 'rounded bg-muted px-1 font-mono text-[10px]'
                      }
                    >
                      {status || 'todo'}
                    </span>
                    <span className="truncate font-mono text-xs">{String(subject?.id?.slug ?? '?')}</span>
                    {item.doc ? (
                      <span className="ml-auto truncate text-[10px] text-muted-foreground">{String(item.doc)}</span>
                    ) : null}
                  </div>
                );
              })}
            </div>
          </section>
        ) : null}
        {entity.kind === 'readModel'
          ? (() => {
              const roads = model.continuations.filter((c) => {
                const rm = storyboardEntryReadModel(c.to);
                return rm && rm.namespace === entity.id.namespace && rm.slug === entity.id.slug;
              });
              return roads.length > 0 ? (
                <section>
                  <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Opens roads</h3>
                  <div className="space-y-1.5">
                    {roads.map((c) => (
                      <button
                        key={c.to.key}
                        type="button"
                        onClick={() => onSelect(c.to.key)}
                        className="w-full rounded-md border border-amber-300 bg-amber-50 px-2 py-1.5 text-left text-xs hover:bg-amber-100"
                      >
                        <span className="inline-flex items-center gap-1 font-semibold">
                          <Split aria-hidden className="h-3 w-3" />
                          {c.to.title}
                        </span>
                        {c.doc ? <span className="block text-[10px] text-muted-foreground">when {c.doc}</span> : null}
                      </button>
                    ))}
                  </div>
                  <p className="mt-1 text-[10px] text-muted-foreground">
                    Observing this state is how the road begins; the {roads[0].afterEvent.slug} stream decides which one
                    wins.
                  </p>
                </section>
              ) : null;
            })()
          : null}
        {entity.kind === 'swimlane' && Array.isArray(entity.raw.transitions) && entity.raw.transitions.length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Lifecycle</h3>
            <div className="space-y-1.5">
              {(entity.raw.transitions as Record<string, unknown>[]).map((t, i) => {
                const after = (t.after as { id?: { slug?: string } } | undefined)?.id?.slug ?? '?';
                const next = Array.isArray(t.next) ? (t.next as Record<string, unknown>[]) : [];
                return (
                  // biome-ignore lint/suspicious/noArrayIndexKey: transitions have no stable id
                  <div key={i} className="rounded-md border border-border px-2 py-1.5 text-xs">
                    <div className="font-mono text-[11px]">after {after}</div>
                    <div className="mt-0.5 space-y-0.5">
                      {next.map((n, j) => {
                        const slug = (n.event as { id?: { slug?: string } } | undefined)?.id?.slug ?? '?';
                        return (
                          // biome-ignore lint/suspicious/noArrayIndexKey: branches have no stable id
                          <div key={j} className="flex items-baseline gap-1.5">
                            <span className="text-amber-600">→</span>
                            <span className="font-mono text-[11px]">{slug}</span>
                            {n.doc ? (
                              <span className="text-[10px] text-muted-foreground">when {String(n.doc)}</span>
                            ) : null}
                          </div>
                        );
                      })}
                    </div>
                  </div>
                );
              })}
            </div>
            <p className="mt-1 text-[10px] text-muted-foreground">
              The stream decides the fork: one successor wins per instance.
            </p>
          </section>
        ) : null}
        {entity.kind === 'storyboard' && model.continuations.some((c) => c.from.key === entity.key) ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Forks into</h3>
            <div className="space-y-1.5">
              {model.continuations
                .filter((c) => c.from.key === entity.key)
                .map((c) => (
                  <button
                    key={c.to.key}
                    type="button"
                    onClick={() => onSelect(c.to.key)}
                    className="w-full rounded-md border border-amber-300 bg-amber-50 px-2 py-1.5 text-left text-xs hover:bg-amber-100"
                  >
                    <span className="font-semibold">{c.to.title}</span>
                    {c.doc ? <span className="block text-[10px] text-muted-foreground">when {c.doc}</span> : null}
                  </button>
                ))}
            </div>
            <p className="mt-1 text-[10px] text-muted-foreground">Alternative timelines: one instance follows one.</p>
          </section>
        ) : null}
        {entity.kind === 'storyboard' && model.continuations.some((c) => c.to.key === entity.key) ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Continues from</h3>
            <div className="space-y-1.5">
              {model.continuations
                .filter((c) => c.to.key === entity.key)
                .map((c) => (
                  <button
                    key={c.from.key}
                    type="button"
                    onClick={() => onSelect(c.from.key)}
                    className="w-full rounded-md border border-border px-2 py-1.5 text-left text-xs hover:bg-accent"
                  >
                    <span className="font-semibold">{c.from.title}</span>
                    {c.doc ? <span className="block text-[10px] text-muted-foreground">when {c.doc}</span> : null}
                  </button>
                ))}
            </div>
          </section>
        ) : null}
        {entity.tech && entity.tech.length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Tech</h3>
            <div className="flex flex-wrap gap-1">
              {entity.tech.map((t) => (
                <Badge key={t} variant="outline" className="font-mono">
                  {t}
                </Badge>
              ))}
            </div>
          </section>
        ) : null}
        {entity.kind === 'component' && Array.isArray(entity.raw.members) && entity.raw.members.length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">
              Members ({entity.raw.members.length})
            </h3>
            <div className="overflow-hidden rounded-md border border-border">
              {(entity.raw.members as { kind?: string; id?: { namespace?: string; slug?: string } }[]).map((m, i) => (
                <div
                  // biome-ignore lint/suspicious/noArrayIndexKey: members have no stable id beyond ref
                  key={i}
                  className="flex items-baseline gap-2 border-b border-border px-2 py-1.5 last:border-0"
                >
                  <span className="font-mono text-xs">{String(m.id?.slug ?? '?')}</span>
                  <span className="ml-auto text-[10px] text-muted-foreground">
                    {String(m.kind ?? '')
                      .replace('ENTITY_KIND_', '')
                      .toLowerCase()
                      .replace(/_/g, ' ')}
                  </span>
                </div>
              ))}
            </div>
          </section>
        ) : null}
        {orphanOf(entity) !== undefined ? (
          <section>
            <h3 className="mb-1 flex items-center gap-1 text-xs font-semibold uppercase text-amber-700">
              <TriangleAlert aria-hidden className="h-3 w-3" />
              Orphan
            </h3>
            <div className="rounded-md border border-amber-300 bg-amber-50 px-2 py-1.5 text-xs">
              {orphanOf(entity) || (
                <span className="italic text-amber-700">
                  OrphanAnnotation is set but its doc is empty: ORPHAN_DOC_EMPTY. Fill in the reason or remove the
                  annotation.
                </span>
              )}
            </div>
            <p className="mt-1 text-[10px] text-muted-foreground">
              This entity is intentionally disconnected from any slice in its model (Decision #31). Wire it into a slice
              or keep the doc explaining why.
            </p>
          </section>
        ) : null}
        {listedAnnotations.length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">Annotations</h3>
            <div className="space-y-1.5">
              {listedAnnotations.map((a, i) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: annotations have no stable id
                <AnnotationView key={i} annotation={a} />
              ))}
            </div>
          </section>
        ) : null}
        {sliceScenarios.length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">
              Scenarios ({sliceScenarios.length})
            </h3>
            <div className="space-y-1.5">
              {sliceScenarios.map((s, i) => (
                <ScenarioBlock key={s.id || i} scenario={s} givenApplies={ownSlice?.kind !== 'readModelSlice'} />
              ))}
            </div>
          </section>
        ) : null}
        {entity.fields.length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">
              Fields ({entity.fields.length})
            </h3>
            <div className="overflow-hidden rounded-md border border-border">
              {entity.fields.map((f, i) => (
                <div
                  key={`${f.name ?? i}`}
                  className="flex items-baseline gap-2 border-b border-border px-2 py-1.5 last:border-0"
                >
                  <span className="font-mono text-xs">{String(f.name ?? '?')}</span>
                  {fieldTypeLabel(f) ? (
                    <span className="font-mono text-[10px] text-muted-foreground">{fieldTypeLabel(f)}</span>
                  ) : null}
                  {f.doc ? (
                    <span className="ml-auto max-w-[10rem] truncate text-[10px] text-muted-foreground">
                      {String(f.doc)}
                    </span>
                  ) : null}
                </div>
              ))}
            </div>
          </section>
        ) : null}
        {memberships.length > 0 ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">
              Slices ({memberships.length})
            </h3>
            <ul className="space-y-1">
              {memberships.map((s) => (
                <li key={s.entity.key}>
                  <button
                    type="button"
                    className="w-full rounded-md border border-border px-2 py-1.5 text-left text-xs hover:bg-accent"
                    onClick={() => onSelect(s.entity.key)}
                  >
                    <span className="font-medium">{s.entity.title}</span>
                    <span className="ml-2 text-[10px] text-muted-foreground">
                      {s.kind === 'commandSlice'
                        ? 'command slice'
                        : s.kind === 'readModelSlice'
                          ? 'read model slice'
                          : 'automation slice'}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ) : null}
      </div>
    </aside>
  );
}

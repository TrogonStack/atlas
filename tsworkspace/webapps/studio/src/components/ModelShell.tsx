// The studio's five-view shell for a single EventModel page. A caller may
// still omit the model for direct debugging, but the top-level router no
// longer exposes the old whole-store query route.
import { useHotkey } from '@tanstack/react-hotkeys';
import { ReactFlowProvider } from '@xyflow/react';
import { parseAsArrayOf, parseAsString, parseAsStringLiteral, useQueryState } from 'nuqs';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { ConflictInspector } from '@/components/ConflictInspector';
import { Board, type SelectedEdge, type Selection } from '@/components/canvas/Board';
import { BoardTopRightHost } from '@/components/canvas/BoardTopRight';
import { DomainChartBoard } from '@/components/canvas/DomainChartBoard';
import { PlanBoard } from '@/components/canvas/PlanBoard';
import { ScreensBoard } from '@/components/canvas/ScreensBoard';
import { SequenceBoard } from '@/components/canvas/SequenceBoard';
import type { FocusRequest } from '@/components/canvas/useFocusNode';
import { EdgeInspector } from '@/components/EdgeInspector';
import { ErrorBoundary } from '@/components/ErrorBoundary';
import { Inspector } from '@/components/Inspector';
import { IssuesProvider } from '@/components/IssuesProvider';
import { KIND_SHORT_TO_CAMEL } from '@/components/ModelShell.kindmap';
import { MultiInspector } from '@/components/MultiInspector';
import { Sidebar } from '@/components/Sidebar';
import { TopBar } from '@/components/TopBar';
import { ValidationInspector } from '@/components/ValidationInspector';
import { ValidationSummary } from '@/components/ValidationSummary';
import { api, type BranchDiffEntry, type ServerInfo } from '@/lib/api';
import { branchStatusLookup, useBranchParam } from '@/lib/branch';
import { EMPTY_ISSUE_INDEX, type IssueIndex } from '@/lib/issues';
import type { BoardEdgeData } from '@/lib/layout';
import { buildModel, entityKey, type Model } from '@/lib/model';
import { layoutBoards } from '@/lib/multiboard';
import type { RealtimeStatus } from '@/lib/realtime';
import { invalidateEntityHomes } from '@/lib/useEntityHomes';
import { isSafeNamespace } from '../../shared/safe-namespace.mjs';

/** Stable identity for an edge across layout rebuilds (occurrence ids shift). */
function edgeIdentity(data: BoardEdgeData): string {
  return `${data.source.key}\0${data.relation}\0${data.target.key}`;
}

const VIEWS = ['board', 'sequence', 'ui', 'plan', 'domain'] as const;

// URL query-state parsers (nuqs). Params are cleared when they equal the
// default, so a pristine board view keeps a clean URL.
const viewParser = parseAsStringLiteral(VIEWS).withDefault('board');
const nsParser = parseAsArrayOf(parseAsString).withDefault([]);
const selectedParser = parseAsArrayOf(parseAsString);

export interface ModelShellProps {
  /** Pre-loaded model: bypasses the namespace-filtered fetch. */
  model?: Model;
  /** When set, the namespace picker is hidden (single-model context). */
  scopedTitle?: string;
  /** Back-link rendered next to the title (used by /em/ routes). */
  backHref?: string;
  /**
   * Validator findings for this model. Owned by the caller because
   * validation is scoped to one EventModel id, which only the /em/ route
   * knows. Omitted means the shell makes no claim either way.
   */
  issues?: IssueIndex;
  /**
   * Scoped-mode refetch: called (debounced) when the realtime channel
   * reports a store change so the caller can reload the model it owns.
   * Without it a scoped shell has no way to refresh itself, so the
   * realtime subscription is not started and the badge stays offline.
   */
  onRefetch?: () => void;
}

// The empty model is structurally invariant and small; hoisted out of
// `ModelShell` so it's allocated once per process rather than per render.
const EMPTY_MODEL: Model = buildModel([]);

export function ModelShell({
  model: modelProp,
  scopedTitle,
  backHref,
  issues = EMPTY_ISSUE_INDEX,
  onRefetch,
}: ModelShellProps) {
  const [internalModel, setInternalModel] = useState<Model>(modelProp ?? EMPTY_MODEL);
  const [info, setInfo] = useState<ServerInfo>();
  const scoped = Boolean(modelProp);
  const [branch, setBranch] = useBranchParam();
  const [branchDiffEntries, setBranchDiffEntries] = useState<BranchDiffEntry[]>([]);
  const branchLookup = useMemo(() => branchStatusLookup(branchDiffEntries), [branchDiffEntries]);
  // ?ns= is owned by this shell only in non-scoped mode; scoped mode is
  // owned by the router, we just decorate ?view= and never write ?ns=
  // (the Sidebar, the only writer, isn't rendered there).
  const [nsParam, setNsParam] = useQueryState('ns', nsParser);
  const namespaces = useMemo(
    () => (scoped ? [] : nsParam.map((s) => s.trim()).filter(isSafeNamespace)),
    [scoped, nsParam],
  );
  const [allNamespaces, setAllNamespaces] = useState<string[]>([]);
  const [namespaceLabels, setNamespaceLabels] = useState<Record<string, string>>({});
  const [selectedParam, setSelectedParam] = useQueryState('selected', selectedParser);
  const [focusParam, setFocusParam] = useQueryState('focus');
  // Restore entity selections from the URL on mount so a refresh keeps
  // whatever the user had open in the drawer. Sources, in priority
  // order:
  //   1. ?selected=<key>(,<key>)*: explicit persistence written on
  //      every selection change.
  //   2. ?focus=<kind>:<ns>/<slug>: the cross-EM jump's deep-link;
  //      resolve against the prop model so the Inspector renders the
  //      focused entity on the first paint (not after the effect runs
  //      one tick later).
  // Edge selections aren't persisted (edge ids are layout-dependent).
  const [selections, setSelections] = useState<Selection[]>(() => {
    if (selectedParam && selectedParam.length > 0) {
      return selectedParam.filter(Boolean).map<Selection>((key) => ({ type: 'entity', key }));
    }
    if (focusParam && modelProp) {
      const m = focusParam.match(/^([a-z_]+):([^/]+)\/(.+)$/);
      if (m) {
        const [, kindShort, ns, slug] = m;
        const kind = KIND_SHORT_TO_CAMEL[kindShort];
        const target = kind
          ? modelProp.entities.find((e) => e.kind === kind && e.id.namespace === ns && e.id.slug === slug)
          : undefined;
        if (target) return [{ type: 'entity', key: target.key }];
      }
    }
    return [];
  });
  const [view, setView] = useQueryState('view', viewParser);
  const [error, setError] = useState<string>();
  const [validationOpen, setValidationOpen] = useState(false);
  const validationPanelId = useId();
  const validationTrigger = useRef<HTMLButtonElement>(null);
  const closeValidation = useCallback(() => {
    setValidationOpen(false);
    validationTrigger.current?.focus();
  }, []);

  // Entity selections persist as ?selected=<key>(,<key>)* so a refresh
  // restores the drawer to what the user was reading. Selections carry
  // more than keys (edges, occurrence pins), so the state stays local
  // and only this projection is mirrored to the URL.
  useEffect(() => {
    const selectedKeys = selections
      .filter((s): s is Extract<Selection, { type: 'entity' }> => s.type === 'entity')
      .map((s) => s.key);
    setSelectedParam(selectedKeys.length > 0 ? selectedKeys : null);
  }, [selections, setSelectedParam]);

  // Initialize focusReq from the URL at mount so a refresh / deep-link
  // pans the canvas to the entity, not just opens the drawer. Sources
  // in priority order:
  //   1. ?focus=<kind>:<ns>/<slug>: explicit deep-link from a
  //      cross-EM jump. Stored as a placeholder; the entity key is
  //      resolved once the model exposes the target.
  //   2. ?selected=<key>(,<key>)*: refresh-restore path. Pick the
  //      first key as the pan target so the canvas re-centers on
  //      whatever the user was reading.
  // Both result in focusReq being truthy from the first render, which
  // also makes the Board skip its automatic fitView (the pan would
  // otherwise be clobbered a few microtasks later).
  const [focusReq, setFocusReq] = useState<FocusRequest | undefined>(() => {
    if (focusParam) return { pending: true as const, n: 0 };
    const firstKey = selectedParam?.filter(Boolean)[0];
    return firstKey ? { key: firstKey, n: 0 } : undefined;
  });
  const selectEntity = useCallback(
    (key: string | undefined, occId?: string, additive?: boolean, instanceUid?: string, momentKey?: string) => {
      setValidationOpen(false);
      setSelections((prev) => {
        if (!key) return [];
        const entry: Selection = { type: 'entity', key, occId, instanceUid, momentKey };
        if (!additive) return [entry];
        if (prev.some((s) => s.type === 'entity' && s.key === key))
          return prev.filter((s) => !(s.type === 'entity' && s.key === key));
        return [...prev, entry];
      });
    },
    [],
  );
  const selectAndFocus = useCallback(
    (key: string | undefined, occId?: string, additive?: boolean) => {
      selectEntity(key, occId, additive);
      if (key) setFocusReq((prev) => ({ key, n: (prev?.n ?? 0) + 1 }));
    },
    [selectEntity],
  );
  const selectEdge = useCallback((edge: SelectedEdge, additive?: boolean) => {
    setValidationOpen(false);
    setSelections((prev) => {
      const entry: Selection = { type: 'edge', id: edge.id, data: edge.data };
      if (!additive) {
        const isSole = prev.length === 1 && prev[0].type === 'edge' && prev[0].id === edge.id;
        return isSole ? [] : [entry];
      }
      if (prev.some((s) => s.type === 'edge' && s.id === edge.id))
        return prev.filter((s) => !(s.type === 'edge' && s.id === edge.id));
      return [...prev, entry];
    });
  }, []);
  const [realtime, setRealtime] = useState<RealtimeStatus>({ state: 'connecting' });

  // Esc closes the drawer. @tanstack/hotkeys defaults Escape to
  // ignoreInputs: false so it still fires while Sidebar search is
  // focused (text inputs have no native Esc-clear). The callback
  // keeps the previous array identity when nothing is selected so
  // React skips a useless re-render.
  useHotkey('Escape', () => {
    if (validationOpen) {
      closeValidation();
      return;
    }
    setSelections((prev) => (prev.length === 0 ? prev : []));
  });

  const refreshAbortRef = useRef<AbortController | undefined>(undefined);

  // In scoped mode the model is provided by the caller; otherwise the
  // legacy namespace-filtered fetch is used.
  const refresh = useCallback(
    async (ns: string[]) => {
      if (scoped) return;
      refreshAbortRef.current?.abort();
      const ctrl = new AbortController();
      refreshAbortRef.current = ctrl;
      try {
        const res = await api.model(ns, { signal: ctrl.signal, branch });
        if (ctrl.signal.aborted) return;
        setInternalModel(buildModel(res.entities ?? []));
        setError(undefined);
      } catch (e) {
        if (e instanceof Error && e.name === 'AbortError') return;
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [scoped, branch],
  );

  useEffect(() => {
    return () => {
      refreshAbortRef.current?.abort();
    };
  }, []);

  // Whenever the caller-supplied model changes (e.g. EventModelPage swapped
  // to a different model), reflect it.
  useEffect(() => {
    if (modelProp) setInternalModel(modelProp);
  }, [modelProp]);

  // Entity selections re-resolve via byKey on every render. Edge selections
  // snapshot BoardEdgeData at click time, so a refetch that updates edge
  // doc/metadata (or endpoint titles) would leave EdgeInspector stale.
  // Re-bind open edge selections from a fresh layout when the model changes.
  useEffect(() => {
    setSelections((prev) => {
      if (!prev.some((s) => s.type === 'edge')) return prev;
      const { edges } = layoutBoards(internalModel);
      const byIdentity = new Map<string, { id: string; data: BoardEdgeData }>();
      for (const e of edges) {
        const data = e.data as BoardEdgeData | undefined;
        if (!data?.source?.key || !data?.target?.key) continue;
        byIdentity.set(edgeIdentity(data), { id: e.id, data });
      }
      let changed = false;
      const next: Selection[] = [];
      for (const s of prev) {
        if (s.type !== 'edge') {
          next.push(s);
          continue;
        }
        const fresh = byIdentity.get(edgeIdentity(s.data));
        if (!fresh) {
          changed = true;
          continue;
        }
        if (
          fresh.id !== s.id ||
          fresh.data.doc !== s.data.doc ||
          fresh.data.relation !== s.data.relation ||
          fresh.data.source !== s.data.source ||
          fresh.data.target !== s.data.target ||
          fresh.data.metadata !== s.data.metadata
        ) {
          changed = true;
          next.push({ type: 'edge', id: fresh.id, data: fresh.data });
        } else {
          next.push(s);
        }
      }
      return changed ? next : prev;
    });
  }, [internalModel]);

  // Cross-EM jump: when the URL carries ?focus=<kind>:<ns>/<slug> (set by
  // the Inspector's "Open in <em>" links on cross-context seams), focus
  // and select that entity once the model is loaded. The param is then
  // cleared from the URL so a refresh doesn't re-trigger.
  const focusParamConsumedRef = useRef(false);

  useEffect(() => {
    const ctrl = new AbortController();
    setError(undefined);
    api
      .info({ signal: ctrl.signal })
      .then((result) => {
        setInfo(result);
        setError(undefined);
      })
      .catch((e) => {
        if (e instanceof Error && e.name === 'AbortError') return;
        setError(e instanceof Error ? e.message : String(e));
      });
    if (!scoped)
      api
        .namespaces({ signal: ctrl.signal, branch })
        .then((r) => {
          setAllNamespaces(r.namespaces ?? []);
          setNamespaceLabels(r.labels ?? {});
        })
        .catch((e) => {
          if (e instanceof Error && e.name === 'AbortError') return;
          console.warn('failed to load namespaces', e);
          setError(e instanceof Error ? e.message : String(e));
        });
    return () => ctrl.abort();
  }, [scoped, branch]);

  useEffect(() => {
    if (!scoped) refresh(namespaces);
  }, [namespaces, refresh, scoped]);

  // BADGES (Phase 3: Studio), when a branch is active, fetch its diff
  // so Board/SequenceBoard can stamp `branchStatus` onto matching stickies.
  // Cleared on baseline (branch undefined) so switching back drops badges
  // immediately rather than showing stale ones from the last branch.
  // Also re-fetched from the realtime sink so badges stay current after
  // store mutations while a branch preview is open.
  const branchDiffAbortRef = useRef<AbortController | undefined>(undefined);
  const loadBranchDiff = useCallback((activeBranch: string | undefined) => {
    branchDiffAbortRef.current?.abort();
    if (!activeBranch) {
      setBranchDiffEntries([]);
      return;
    }
    const ctrl = new AbortController();
    branchDiffAbortRef.current = ctrl;
    api
      .branchDiff(activeBranch, { signal: ctrl.signal })
      .then((res) => {
        if (ctrl.signal.aborted) return;
        setBranchDiffEntries(res.entries ?? []);
      })
      .catch((e) => {
        if (ctrl.signal.aborted) return;
        console.warn('failed to load branch diff', e);
      });
  }, []);

  useEffect(() => {
    loadBranchDiff(branch);
    return () => branchDiffAbortRef.current?.abort();
  }, [branch, loadBranchDiff]);

  // Scoped shells own the model via the caller. When ?branch= changes the
  // badges update above, but the caller's model must also reload so the
  // board shows the branch overlay rather than a stale baseline snapshot.
  const onRefetchRef = useRef(onRefetch);
  onRefetchRef.current = onRefetch;
  const scopedBranchReadyRef = useRef(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: branch is the trigger, not a read
  useEffect(() => {
    if (!scoped) return;
    if (!scopedBranchReadyRef.current) {
      scopedBranchReadyRef.current = true;
      return;
    }
    onRefetchRef.current?.();
  }, [branch, scoped]);

  // Realtime: the browser opens a WebSocket directly to NATS and watches
  // the trogon-atlas-entities KV bucket. Any mutation anywhere triggers a
  // refetch of the active model. Mutations still flow through the
  // gateway → server → store; this channel is read-only push.
  //
  // The subscription is established once per mount; `namespaces` /
  // `refresh` / `scoped` are read through refs so a namespace toggle does
  // not tear down and recreate the NATS connection (which would burst
  // connect/close cycles on a recovering broker).
  //
  // Multi-tab note: `watchEntities` elects one leader per origin (via
  // BroadcastChannel) so only one tab holds the NATS socket. Each tab
  // still issues its own `refresh(namespaces)` because the namespace
  // filter is tab-local; deduping the fetch itself would require
  // broadcasting the resulting model, which costs more than the fetch.
  const refetchSinkRef = useRef<() => void>(() => {});
  refetchSinkRef.current = () => {
    if (!scoped) refresh(namespaces);
    else onRefetchRef.current?.();
    loadBranchDiff(branch);
  };
  useEffect(() => {
    // Scoped shells subscribe only when the caller supplied onRefetch;
    // without a way to reload the caller-owned model, a live channel
    // would report changes the board could never reflect.
    if (scoped && !onRefetchRef.current) {
      setRealtime({ state: 'offline', detail: 'Automatic updates are unavailable for this view.' });
      return;
    }
    let handle: { close(): void } | undefined;
    let disposed = false;
    let pending: number | undefined;
    const debouncedRefetch = () => {
      invalidateEntityHomes();
      if (pending !== undefined) return;
      pending = window.setTimeout(() => {
        pending = undefined;
        if (disposed) return;
        refetchSinkRef.current();
      }, 250);
    };
    (async () => {
      try {
        const { watchEntities } = await import('@/lib/realtime');
        if (disposed) return;
        const subscription = await watchEntities(debouncedRefetch, (state, detail) => {
          if (disposed) return;
          setRealtime({ state, detail });
        });
        if (disposed) subscription.close();
        else handle = subscription;
      } catch (err) {
        if (!disposed) setRealtime({ state: 'offline', detail: err instanceof Error ? err.message : String(err) });
        if (import.meta.env.DEV) {
          console.error('realtime watch failed', err);
        }
      }
    })();
    return () => {
      disposed = true;
      if (pending !== undefined) window.clearTimeout(pending);
      handle?.close();
    };
  }, [scoped]);

  const model = internalModel;
  // Consume ?focus= once the model carries the target entity. Format:
  // <kind>:<namespace>/<slug>  e.g.  event:market-period/trade.settled
  useEffect(() => {
    if (focusParamConsumedRef.current) return;
    if (!focusParam) return;
    const m = focusParam.match(/^([a-z_]+):([^/]+)\/(.+)$/);
    if (!m) return;
    const [, kindShort, ns, slug] = m;
    const kind = KIND_SHORT_TO_CAMEL[kindShort];
    if (!kind) return;
    const target = model.entities.find((e) => e.kind === kind && e.id.namespace === ns && e.id.slug === slug);
    if (!target) return;
    focusParamConsumedRef.current = true;
    selectAndFocus(target.key);
    setFocusParam(null);
  }, [model, selectAndFocus, focusParam, setFocusParam]);

  // A view with nothing to draw is not offered: an EM without UI screens
  // (or slices for the plan, or knowledge entities for the domain chart)
  // would put an empty canvas behind a clickable tab. Board and sequence
  // always show: they are the model itself.
  const availableViews = VIEWS.filter((v) => {
    if (v === 'ui') return model.entities.some((e) => e.kind === 'ui');
    if (v === 'plan')
      return model.entities.some(
        (e) =>
          e.kind === 'commandSlice' ||
          e.kind === 'readModelSlice' ||
          e.kind === 'automationSlice' ||
          e.kind === 'uiSlice',
      );
    if (v === 'domain')
      return model.entities.some((e) => e.kind === 'domain' || e.kind === 'subdomain' || e.kind === 'boundedContext');
    return true;
  });
  // Deep links to a hidden view (?view=ui on a UI-less EM) render the
  // board instead; the param is left alone so it starts working again
  // the moment the model gains that content.
  const activeView = availableViews.includes(view) ? view : 'board';

  const selectedEntities = selections
    .filter((s): s is Extract<Selection, { type: 'entity' }> => s.type === 'entity')
    .map((s) => model.byKey.get(s.key))
    .filter((e): e is NonNullable<typeof e> => Boolean(e));
  const selectedEdges = selections.filter((s): s is Extract<Selection, { type: 'edge' }> => s.type === 'edge');
  // Drawer mode must follow RESOLVED selections. URL-restored keys that no
  // longer exist in the model (deleted entity, switched namespace) leave
  // `selections.length` inflated; keying off it opens MultiInspector with a
  // single row instead of the normal Inspector / EdgeInspector.
  const resolvedCount = selectedEntities.length + selectedEdges.length;
  const soleEntity = resolvedCount === 1 && selectedEntities.length === 1 ? selectedEntities[0] : undefined;
  const soleSelection =
    soleEntity && selections.length >= 1
      ? selections.find(
          (s): s is Extract<Selection, { type: 'entity' }> => s.type === 'entity' && s.key === soleEntity.key,
        )
      : undefined;
  const soleEdge = resolvedCount === 1 && selectedEdges.length === 1 ? selectedEdges[0] : undefined;
  const [reviewOpen, setReviewOpen] = useState(false);

  return (
    <div className="flex h-screen flex-col">
      <TopBar
        info={info}
        realtime={realtime}
        error={error}
        onRefresh={() => (scoped ? onRefetch?.() : refresh(namespaces))}
        scopedTitle={scopedTitle}
        backHref={backHref}
        branch={branch}
        onSelectBranch={(next) => {
          setBranch(next);
          setReviewOpen(false);
          setValidationOpen(false);
        }}
        onOpenBranchReview={() => {
          setValidationOpen(false);
          setReviewOpen(true);
        }}
      />
      <div className="flex min-h-0 flex-1">
        {scoped ? null : (
          <Sidebar
            model={model}
            namespaces={namespaces}
            allNamespaces={allNamespaces}
            namespaceLabels={namespaceLabels}
            branch={branch}
            onNamespaces={(ns) => {
              setNsParam(ns.length > 0 ? ns : null);
              selectEntity(undefined);
            }}
            onSelect={selectAndFocus}
          />
        )}
        <main className="relative min-w-0 flex-1">
          <BoardTopRightHost
            corner={
              <ValidationSummary
                issues={issues}
                open={validationOpen}
                panelId={validationPanelId}
                triggerRef={validationTrigger}
                onToggle={() => {
                  setReviewOpen(false);
                  setValidationOpen((open) => !open);
                }}
              />
            }
          >
            <div className="absolute left-2 top-2 z-10 flex overflow-hidden rounded-md border border-border bg-card text-xs shadow-sm">
              {availableViews.map((v) => (
                <button
                  key={v}
                  type="button"
                  onClick={() => setView(v)}
                  className={`px-3 py-1.5 font-medium capitalize ${activeView === v ? 'bg-foreground text-background' : 'hover:bg-accent'}`}
                >
                  {v === 'board'
                    ? 'Board'
                    : v === 'sequence'
                      ? 'Sequence'
                      : v === 'ui'
                        ? 'UI'
                        : v === 'plan'
                          ? 'Plan'
                          : 'Domain'}
                </button>
              ))}
            </div>
            <IssuesProvider issues={issues}>
              <ReactFlowProvider>
                {activeView === 'board' ? (
                  <ErrorBoundary>
                    <Board
                      model={model}
                      selections={selections}
                      onSelect={selectEntity}
                      onSelectEdge={selectEdge}
                      focus={focusReq}
                      branchStatusLookup={branchLookup}
                    />
                  </ErrorBoundary>
                ) : activeView === 'sequence' ? (
                  <ErrorBoundary>
                    <SequenceBoard
                      model={model}
                      selections={selections}
                      onSelect={selectEntity}
                      onSelectEdge={selectEdge}
                      focus={focusReq}
                      branchStatusLookup={branchLookup}
                    />
                  </ErrorBoundary>
                ) : activeView === 'ui' ? (
                  <ErrorBoundary>
                    <ScreensBoard
                      model={model}
                      selections={selections}
                      onSelect={selectEntity}
                      onSelectEdge={selectEdge}
                      focus={focusReq}
                    />
                  </ErrorBoundary>
                ) : activeView === 'plan' ? (
                  <ErrorBoundary>
                    <PlanBoard model={model} selections={selections} onSelect={selectEntity} focus={focusReq} />
                  </ErrorBoundary>
                ) : (
                  <ErrorBoundary>
                    <DomainChartBoard model={model} selections={selections} onSelect={selectEntity} focus={focusReq} />
                  </ErrorBoundary>
                )}
              </ReactFlowProvider>
            </IssuesProvider>
          </BoardTopRightHost>
        </main>
        {validationOpen ? (
          <ValidationInspector
            id={validationPanelId}
            model={model}
            issues={issues}
            onClose={closeValidation}
            canSelect={(subject) => model.byKey.has(entityKey(subject.kind, subject.id))}
            onSelect={(subject) => {
              closeValidation();
              selectAndFocus(entityKey(subject.kind, subject.id));
            }}
          />
        ) : reviewOpen && branch ? (
          <ConflictInspector branch={branch} entries={branchDiffEntries} onClose={() => setReviewOpen(false)} />
        ) : soleEntity ? (
          <Inspector
            model={model}
            entity={soleEntity}
            instanceUid={soleSelection?.instanceUid}
            momentKey={soleSelection?.momentKey}
            issues={issues}
            onClose={() => selectEntity(undefined)}
            onSelect={selectAndFocus}
          />
        ) : soleEdge ? (
          <EdgeInspector edge={soleEdge.data} onClose={() => selectEntity(undefined)} onSelect={selectAndFocus} />
        ) : resolvedCount > 1 ? (
          <MultiInspector
            model={model}
            entities={selectedEntities}
            edges={selectedEdges.map((s) => ({ id: s.id, data: s.data }))}
            issues={issues}
            onClose={() => selectEntity(undefined)}
            onSelect={selectAndFocus}
            onSelectEdge={(edge) => selectEdge(edge, false)}
          />
        ) : null}
      </div>
    </div>
  );
}

import { authHeaders, onUnauthenticated } from '@/lib/credential';
import type { EntityId } from '@/lib/model';

// Hard ceiling on every browser → gateway request. Long-running RPCs are
// expected to stream (SSE / NATS WS); a 30 s wall clock for a single JSON
// fetch covers the slowest expected listEntities sweep with margin.
const DEFAULT_TIMEOUT_MS = 30_000;

export interface FetchOptions {
  signal?: AbortSignal;
  timeoutMs?: number;
  /**
   * Branch preview context. When set, forwarded to the bridge as `?branch=`
   * and as the `x-trogon-atlas-branch` request header (the bridge re-emits the
   * query as gRPC metadata). Omitted → baseline, byte-identical to
   * pre-branching behavior.
   */
  branch?: string;
}

export interface ChangesFetchOptions extends FetchOptions {
  /** Forwarded as `?pageSize=` for ListChanges. Server default is 100. */
  pageSize?: number;
}

function withBranchParam(url: string, branch: string | undefined): string {
  if (!branch) return url;
  const [base, query] = url.split('?');
  const params = new URLSearchParams(query ?? '');
  params.set('branch', branch);
  return `${base}?${params.toString()}`;
}

function branchHeaders(branch: string | undefined): Record<string, string> {
  return branch ? { 'x-trogon-atlas-branch': branch } : {};
}

/** Thrown on a 401 so callers can tell "wrong key" from "backend is down". */
export class UnauthenticatedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'UnauthenticatedError';
  }
}

type UnauthenticatedListener = (message: string) => void;
const unauthenticatedListeners = new Set<UnauthenticatedListener>();

/**
 * Notified whenever the bridge refuses the credential we sent. Lives here
 * rather than in each page because any request can be the one that finds out,
 * and every one of them should raise the same prompt.
 */
export function subscribeUnauthenticated(listener: UnauthenticatedListener): () => void {
  unauthenticatedListeners.add(listener);
  return () => {
    unauthenticatedListeners.delete(listener);
  };
}

/**
 * Raise the same prompt for a refusal that did not arrive through
 * `fetchJson`. The realtime stream reads its own response, so it never
 * passes through the check below, and a browser whose key stopped working
 * would otherwise see a feed that is quietly empty rather than a prompt.
 */
export function reportUnauthenticated(message: string): void {
  onUnauthenticated();
  for (const listener of [...unauthenticatedListeners]) listener(message);
}

function rejectIfUnauthenticated(res: Response, message: string): void {
  if (res.status !== 401) return;
  reportUnauthenticated(message);
  throw new UnauthenticatedError(message);
}

async function readErrorMessage(res: Response): Promise<string> {
  const text = await res.text().catch(() => '');
  if (text) {
    try {
      const body = JSON.parse(text) as { error?: unknown; message?: unknown };
      if (typeof body.error === 'string' && body.error.length > 0) return body.error;
      if (typeof body.message === 'string' && body.message.length > 0) return body.message;
      // Parsed JSON without a usable message: fall back to status line.
    } catch {
      // Not JSON: use the raw body text.
      const trimmed = text.trim();
      if (trimmed.length > 0) return trimmed.slice(0, 500);
    }
  }
  return `${res.status} ${res.statusText}`;
}

function isTimeoutAbort(timeoutSignal: AbortSignal, callerSignal: AbortSignal | undefined): boolean {
  return timeoutSignal.aborted && !callerSignal?.aborted;
}

async function getJson<T>(url: string, opts: FetchOptions = {}): Promise<T> {
  // Compose the caller's signal (if any) with our internal timeout so an
  // unmount-during-fetch cancels the network call, not just the rendered
  // state. Native AbortSignal.any is widely supported but not universal yet,
  // so fall back to a manual relay when missing.
  const timeoutMs = opts.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const timeoutCtrl = new AbortController();
  const timer =
    timeoutMs > 0 ? window.setTimeout(() => timeoutCtrl.abort(new Error('request timed out')), timeoutMs) : null;
  const signal = combineSignals([opts.signal, timeoutCtrl.signal]);
  try {
    const res = await fetch(withBranchParam(url, opts.branch), {
      signal,
      headers: { ...branchHeaders(opts.branch), ...authHeaders() },
    });
    if (!res.ok) {
      const message = await readErrorMessage(res);
      rejectIfUnauthenticated(res, message);
      throw new Error(message);
    }
    return (await res.json()) as T;
  } catch (e) {
    // Browser fetch rejects timeouts as AbortError and drops the abort
    // reason, which callers then treat as "unmounted / cancelled" and
    // silently ignore. Re-throw a plain Error so the timeout message
    // survives ModelShell/Sidebar AbortError filters.
    if (isTimeoutAbort(timeoutCtrl.signal, opts.signal)) {
      throw new Error('request timed out');
    }
    throw e;
  } finally {
    if (timer != null) window.clearTimeout(timer);
  }
}

async function postJson<T>(url: string, body: unknown, opts: FetchOptions = {}): Promise<T> {
  const timeoutMs = opts.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const timeoutCtrl = new AbortController();
  const timer =
    timeoutMs > 0 ? window.setTimeout(() => timeoutCtrl.abort(new Error('request timed out')), timeoutMs) : null;
  const signal = combineSignals([opts.signal, timeoutCtrl.signal]);
  try {
    const res = await fetch(withBranchParam(url, opts.branch), {
      method: 'POST',
      signal,
      headers: { 'Content-Type': 'application/json', ...branchHeaders(opts.branch), ...authHeaders() },
      body: JSON.stringify(body),
    });
    if (!res.ok) {
      const message = await readErrorMessage(res);
      rejectIfUnauthenticated(res, message);
      throw new Error(message);
    }
    return (await res.json()) as T;
  } catch (e) {
    if (isTimeoutAbort(timeoutCtrl.signal, opts.signal)) {
      throw new Error('request timed out');
    }
    throw e;
  } finally {
    if (timer != null) window.clearTimeout(timer);
  }
}

interface AbortSignalWithAny {
  any(signals: AbortSignal[]): AbortSignal;
}

function hasAbortSignalAny(s: typeof AbortSignal): s is typeof AbortSignal & AbortSignalWithAny {
  return typeof (s as Partial<AbortSignalWithAny>).any === 'function';
}

function combineSignals(signals: (AbortSignal | undefined)[]): AbortSignal {
  const active = signals.filter((s): s is AbortSignal => Boolean(s));
  if (active.length === 0) return new AbortController().signal;
  if (active.length === 1) return active[0];
  if (hasAbortSignalAny(AbortSignal)) return AbortSignal.any(active);
  const ctrl = new AbortController();
  for (const s of active) {
    if (s.aborted) ctrl.abort(s.reason);
    else s.addEventListener('abort', () => ctrl.abort(s.reason), { once: true });
  }
  return ctrl.signal;
}

export interface ServerInfo {
  schemaVersion?: string;
  serverVersion?: string;
  features?: Record<string, boolean>;
}

export interface WireEntityEnvelope {
  kind?: string;
  [key: string]: unknown;
}

export interface SearchHit {
  entity?: WireEntityEnvelope;
  score?: number;
  excerpt?: string;
}

export interface ChangesPage {
  events?: { kind?: string; entity?: unknown; at?: string; token?: string }[];
  nextToken?: string;
}

export interface BranchInfo {
  name: string;
  doc: string;
  createdAt: string;
  deltaCount: number;
}

export interface EntityRefWire {
  kind?: string;
  id?: { namespace?: string; slug?: string; version?: string };
}

export interface BranchDiffEntry {
  ref: EntityRefWire | null;
  status: string;
  base: WireEntityEnvelope | null;
  baseEtag: string;
  ours: WireEntityEnvelope | null;
  theirs: WireEntityEnvelope | null;
  conflictFieldPaths: string[];
}

export interface ValidateProjectScope {
  projectNamespace?: string;
  projectSlug?: string;
  projectVersion?: string;
  domainNamespace?: string;
  domainSlug?: string;
  domainVersion?: string;
}

function buildQuery(params: Record<string, string | number | undefined>): string {
  const sp = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === '') continue;
    sp.set(key, String(value));
  }
  const qs = sp.toString();
  return qs ? `?${qs}` : '';
}

// `info` deliberately never takes a `branch`: it reports server/schema
// version, which is identical regardless of branch. Everything else does:
// `search` was once excluded on the strength of a proto comment saying the
// server ignored branch metadata, which stopped being true once search grew
// a per-branch index. Omitting the branch made a search during a branch preview answer from baseline, which
// reads as the branch having lost the entity that was just added to it.
export interface NamespaceRegistryRow {
  /** The NamespaceId entity keys are built from. Empty when unregistered. */
  id: string;
  name: string;
  /** OwnerId. Empty means the namespace has no owner recorded. */
  parent: string;
  entityCount: number;
  registered: boolean;
}

export interface ProtoSourceFile {
  path: string;
  content: string;
}

export interface TypeLibraryWire {
  id: EntityId;
  files: ProtoSourceFile[];
  dependencies: { id: EntityId }[];
}

export interface TypeLibraryDiagnostic {
  path?: string;
  line?: number;
  column?: number;
  message?: string;
}

export interface CompileTypeLibraryResult {
  diagnostics?: TypeLibraryDiagnostic[];
  compatibilityViolations?: { message?: string }[];
}

export interface TypeLibraryMessage {
  fullName: string;
  library?: EntityId;
}

export const api = {
  info: (opts?: Omit<FetchOptions, 'branch'>) => getJson<ServerInfo>('/api/info', opts),
  // `namespaces` are NamespaceIds, which is what every other endpoint
  // filters on; `labels` maps each to the name to show a person.
  namespaces: (opts?: FetchOptions) =>
    getJson<{ namespaces: string[]; labels?: Record<string, string> }>('/api/namespaces', opts),
  // The registry behind the picker. `namespaces` above collapses `id || name`
  // so every row has something to filter on; this one keeps them apart,
  // because a row with no id is a namespace the directory hides from every
  // scoped caller and that is the whole reason to look at this screen.
  namespaceRegistry: (opts?: FetchOptions) =>
    getJson<{ namespaces: NamespaceRegistryRow[] }>('/api/namespaces/registry', opts),
  model: (namespaces?: string[], opts?: FetchOptions) =>
    getJson<{ entities: WireEntityEnvelope[] }>(
      namespaces && namespaces.length > 0
        ? `/api/model?namespace=${encodeURIComponent(namespaces.join(','))}`
        : '/api/model',
      opts,
    ),
  search: (q: string, namespaces?: string[], opts?: FetchOptions) =>
    getJson<{ results?: SearchHit[] }>(
      `/api/search?q=${encodeURIComponent(q)}${
        namespaces && namespaces.length > 0 ? `&namespace=${encodeURIComponent(namespaces.join(','))}` : ''
      }`,
      opts,
    ),
  changes: (after: string, opts?: ChangesFetchOptions) => {
    const qs = buildQuery({
      after,
      pageSize: opts?.pageSize,
    });
    return getJson<ChangesPage>(`/api/changes${qs}`, opts);
  },
  overview: (opts?: FetchOptions) => getJson<{ entities: WireEntityEnvelope[] }>('/api/overview', opts),
  eventModels: (opts?: FetchOptions) => getJson<{ entities: WireEntityEnvelope[] }>('/api/event-models', opts),
  eventModel: (namespace: string, slug: string, opts?: FetchOptions) =>
    getJson<{ entities: WireEntityEnvelope[] }>(
      `/api/event-model/${encodeURIComponent(namespace)}/${encodeURIComponent(slug)}`,
      opts,
    ),
  branches: (opts?: Omit<FetchOptions, 'branch'>) => getJson<{ branches: BranchInfo[] }>('/api/branches', opts),
  branchDiff: (name: string, opts?: Omit<FetchOptions, 'branch'>) =>
    getJson<{ entries: BranchDiffEntry[] }>(`/api/branch-diff?name=${encodeURIComponent(name)}`, opts),
  validate: (id: { namespace: string; slug: string; version?: string }, opts?: FetchOptions) =>
    getJson<{ issues?: unknown[] }>(
      `/api/validate${buildQuery({
        namespace: id.namespace,
        slug: id.slug,
        version: id.version,
      })}`,
      opts,
    ),
  compileTypeLibrary: (library: TypeLibraryWire, opts?: FetchOptions) =>
    postJson<CompileTypeLibraryResult>('/api/type-libraries/compile', { library }, opts),
  typeMessages: (namespace: string, opts?: FetchOptions) =>
    getJson<{ messages?: TypeLibraryMessage[] }>(`/api/type-libraries/messages${buildQuery({ namespace })}`, opts),
  validateProject: (scope: ValidateProjectScope, opts?: FetchOptions) =>
    getJson<{ reports?: unknown[]; totalErrors?: number; totalWarnings?: number; totalInfo?: number }>(
      `/api/validate-project${buildQuery({
        projectNamespace: scope.projectNamespace,
        projectSlug: scope.projectSlug,
        projectVersion: scope.projectVersion,
        domainNamespace: scope.domainNamespace,
        domainSlug: scope.domainSlug,
        domainVersion: scope.domainVersion,
      })}`,
      opts,
    ),
};

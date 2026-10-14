// Saves the open board as one HTML file: the snapshot runtime from
// build:snapshot plus the board's read responses, inlined so the file opens
// from disk with no backend. Only the reads the board page makes are
// recorded; realtime and credential endpoints are never part of a snapshot.
import { api, type BoardManifests, boardPaths } from '@/lib/api';
import { download } from '@/lib/download';
import { SNAPSHOT_VERSION, type SnapshotManifests, type StudioSnapshot, snapshotFileName } from '@/lib/snapshot';

const RUNTIME_JS = '/snapshot/runtime.js';
const RUNTIME_CSS = '/snapshot/runtime.css';

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/**
 * Inline `js`, `css` and `snapshot` into one document. The browser ends an
 * inline script at the first `</script` and treats `<!--` inside it
 * specially, whatever JS surrounds them, so the runtime writes the `<` of
 * both as `\x3C`, which reads the same in JS strings, templates and regexes.
 * The JSON writes every `<` as `\u003c` for the same reason.
 */
export function buildSnapshotHtml({
  js,
  css,
  snapshot,
  title,
}: {
  js: string;
  css: string;
  snapshot: StudioSnapshot;
  title: string;
}): string {
  const data = JSON.stringify(snapshot).replace(/</g, '\\u003c');
  const script = js.replace(/<(!--|\/script)/gi, '\\x3C$1');
  const style = css.replace(/<\/(style)/gi, '<\\/$1');
  return [
    '<!doctype html>',
    '<html lang="en">',
    '<head>',
    '<meta charset="UTF-8" />',
    '<meta name="viewport" content="width=device-width, initial-scale=1.0" />',
    `<title>${escapeHtml(title)} (snapshot)</title>`,
    `<style>${style}</style>`,
    '</head>',
    '<body>',
    '<div id="root"></div>',
    `<script id="atlas-snapshot" type="application/json">${data}</script>`,
    `<script type="module">${script}</script>`,
    '</body>',
    '</html>',
    '',
  ].join('\n');
}

async function fetchRuntime(path: string): Promise<string> {
  const res = await fetch(path);
  // A server without the build answers a missing file with the app's HTML shell.
  if (!res.ok || res.headers.get('content-type')?.startsWith('text/html')) {
    throw new Error('export needs the snapshot runtime: run build:snapshot');
  }
  return res.text();
}

let runtime: Promise<[string, string]> | undefined;

/** The runtime does not change while Studio is open, so it is fetched once. */
function loadRuntime(): Promise<[string, string]> {
  runtime ??= Promise.all([fetchRuntime(RUNTIME_JS), fetchRuntime(RUNTIME_CSS)]).catch((e: unknown) => {
    runtime = undefined;
    throw e;
  });
  return runtime;
}

/** Read the board again, as the viewer would see it now, and record the responses. */
async function captureSnapshot({
  namespace,
  slug,
  branch,
}: {
  namespace: string;
  slug: string;
  branch?: string;
}): Promise<StudioSnapshot> {
  const id = { namespace, slug };
  const [info, eventModel, validate, manifests] = await Promise.all([
    api.info(),
    api.eventModel(namespace, slug, { branch }),
    // A validator that is down should not block the export; the snapshot
    // then shows validation as not run, the same as the live page does.
    api.validate(id, { branch }).catch(() => undefined),
    // An older server has no manifests RPC; the board still saves, without YAML.
    api.manifests(namespace, slug, { branch }).catch(() => undefined),
  ]);
  const responses: Record<string, unknown> = {
    [boardPaths.info()]: info,
    [boardPaths.eventModel(namespace, slug)]: eventModel,
  };
  if (validate) responses[boardPaths.validate(id)] = validate;
  const embedded = snapshotManifests(manifests);
  return {
    version: SNAPSHOT_VERSION,
    capturedAt: new Date().toISOString(),
    namespace,
    slug,
    ...(branch ? { branch } : {}),
    responses,
    ...(embedded ? { manifests: embedded } : {}),
  };
}

/**
 * Only a complete board is embedded: applying a clipped or partly rendered
 * one would drop entities with no sign that anything is missing.
 */
export function snapshotManifests(resp: BoardManifests | undefined): SnapshotManifests | undefined {
  if (!resp?.manifests || resp.truncated || resp.skipped?.length) return undefined;
  return { model: resp.manifests, external: resp.externalManifests ?? '' };
}

export async function exportBoardHtml(board: {
  namespace: string;
  slug: string;
  branch?: string;
  title: string;
}): Promise<void> {
  const [snapshot, [js, css]] = await Promise.all([captureSnapshot(board), loadRuntime()]);
  download(snapshotFileName(snapshot), buildSnapshotHtml({ js, css, snapshot, title: board.title }), 'text/html');
}

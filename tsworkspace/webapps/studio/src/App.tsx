// Top-level router. Two routes: Overview, the map, and EventModel, the page
// you read.
//
// No react-router dependency; the pathname + history API are sufficient
// for the navigation depth we have today.
import { lazy, Suspense, useEffect, useState } from 'react';
import { CredentialPrompt } from '@/components/CredentialPrompt';
import { ErrorBoundary } from '@/components/ErrorBoundary';
import { subscribeUnauthenticated } from '@/lib/api';
import { isSafeNamespace } from '../shared/safe-namespace.mjs';

// Route-level code-splitting: each page lands in its own chunk so the
// landing route does not ship the bytes for EventModel pages.
const OverviewPage = lazy(() => import('@/components/OverviewPage').then((m) => ({ default: m.OverviewPage })));
const EventModelPage = lazy(() => import('@/components/EventModelPage').then((m) => ({ default: m.EventModelPage })));
const NamespacesPage = lazy(() => import('@/components/NamespacesPage').then((m) => ({ default: m.NamespacesPage })));

interface Route {
  kind: 'overview' | 'event-model' | 'namespaces';
  namespace?: string;
  slug?: string;
}

function parseLocation(): Route {
  const path = window.location.pathname;
  if (/^\/namespaces\/?$/.test(path)) {
    return { kind: 'namespaces' };
  }
  // /em/<namespace>/<slug> (allow trailing slash)
  const m = path.match(/^\/em\/([^/]+)\/([^/]+)\/?$/);
  if (m) {
    // `decodeURIComponent` throws `URIError` on a malformed `%xx` sequence
    // like `/em/%ZZ/foo`. Falling back to the legacy route keeps the
    // useState initializer total, so a hostile or typo'd URL never lands
    // the app in the ErrorBoundary on the very first paint.
    // The same fallback applies when a decoded segment fails
    // `isSafeNamespace` (leading dot, `;`, etc.): routing those to
    // EventModelPage only surfaces a 400 from the bridge.
    try {
      const namespace = decodeURIComponent(m[1]);
      const slug = decodeURIComponent(m[2]);
      if (!isSafeNamespace(namespace) || !isSafeNamespace(slug)) {
        return { kind: 'overview' };
      }
      return {
        kind: 'event-model',
        namespace,
        slug,
      };
    } catch {
      return { kind: 'overview' };
    }
  }
  return { kind: 'overview' };
}

function RouteLoading() {
  return (
    <output
      aria-live="polite"
      style={{
        display: 'block',
        padding: 32,
        fontFamily: 'system-ui',
        fontSize: 14,
        color: '#52525b',
      }}
    >
      Loading…
    </output>
  );
}

function isChunkLoadError(err: Error): boolean {
  return (
    err.name === 'ChunkLoadError' ||
    /Failed to fetch dynamically imported module/.test(err.message) ||
    /Loading chunk/.test(err.message) ||
    /Importing a module script failed/.test(err.message)
  );
}

function ChunkLoadFallback({ error, reset }: { error: Error; reset: () => void }) {
  return (
    <div
      role="alert"
      style={{
        padding: 32,
        fontFamily: 'system-ui',
        fontSize: 14,
        color: '#52525b',
      }}
    >
      {isChunkLoadError(error) ? (
        <>
          <p style={{ marginTop: 0 }}>A page chunk failed to load. This usually means the app was redeployed.</p>
          <button
            type="button"
            onClick={() => window.location.reload()}
            style={{
              marginTop: 12,
              padding: '6px 12px',
              borderRadius: 6,
              border: '1px solid #d1d5db',
              background: 'white',
              cursor: 'pointer',
            }}
          >
            Reload
          </button>
        </>
      ) : (
        <>
          <p style={{ marginTop: 0 }}>Something went wrong loading this page.</p>
          <pre
            style={{
              background: '#fef2f2',
              padding: 12,
              borderRadius: 6,
              overflow: 'auto',
              fontSize: 12,
            }}
          >
            {error.message}
          </pre>
          <button
            type="button"
            onClick={reset}
            style={{
              marginTop: 12,
              padding: '6px 12px',
              borderRadius: 6,
              border: '1px solid #d1d5db',
              background: 'white',
              cursor: 'pointer',
            }}
          >
            Try again
          </button>
        </>
      )}
    </div>
  );
}

export default function App() {
  const [route, setRoute] = useState<Route>(parseLocation);
  // Set by the first 401 from anywhere in the app, cleared once a key is
  // entered. The pages behind it stay mounted: their next fetch carries the
  // new key, so there is nothing to reload.
  const [authFailure, setAuthFailure] = useState<string | null>(null);

  useEffect(() => {
    const onPop = () => setRoute(parseLocation());
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, []);

  useEffect(() => subscribeUnauthenticated((message) => setAuthFailure(message)), []);

  return (
    <ErrorBoundary fallback={(props) => <ChunkLoadFallback {...props} />}>
      {authFailure !== null ? <CredentialPrompt message={authFailure} onDismiss={() => setAuthFailure(null)} /> : null}
      <Suspense fallback={<RouteLoading />}>
        {route.kind === 'namespaces' ? (
          <NamespacesPage />
        ) : route.kind === 'event-model' && route.namespace && route.slug ? (
          <EventModelPage namespace={route.namespace} slug={route.slug} />
        ) : (
          <OverviewPage />
        )}
      </Suspense>
    </ErrorBoundary>
  );
}

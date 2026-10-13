// Entry for a board saved as one HTML file: the page carries a snapshot in
// <script id="atlas-snapshot" type="application/json">, and every read is
// answered from it. Only the EventModel page is bundled; there is no router.
import { NuqsAdapter } from 'nuqs/adapters/react';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ErrorBoundary } from './components/ErrorBoundary';
import { EventModelPage } from './components/EventModelPage';
import { installSnapshot, parseSnapshot } from './lib/snapshot';
import './index.css';

const root = document.getElementById('root');
if (!root) throw new Error('missing #root element');
const snapshot = parseSnapshot(document.getElementById('atlas-snapshot')?.textContent ?? '');
installSnapshot(snapshot);
createRoot(root).render(
  <StrictMode>
    <ErrorBoundary>
      <NuqsAdapter>
        <EventModelPage namespace={snapshot.namespace} slug={snapshot.slug} />
      </NuqsAdapter>
    </ErrorBoundary>
  </StrictMode>,
);

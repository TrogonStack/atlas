// Top-level error boundary. Renders a recoverable error UI instead of the
// blank-screen failure mode a thrown render gives by default. Composed
// around the page-level routes in `App.tsx`; per-page boundaries can wrap
// `Board` / `OverviewPage` for finer-grained recovery.
import { Component, type ErrorInfo, type ReactNode } from 'react';

interface Props {
  children: ReactNode;
  fallback?: (props: { error: Error; reset: () => void }) => ReactNode;
}

interface State {
  error: Error | null;
}

function toError(error: unknown): Error {
  if (error instanceof Error) return error;
  if (typeof error === 'string') return new Error(error);
  try {
    return new Error(JSON.stringify(error));
  } catch {
    return new Error(String(error));
  }
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: unknown): State {
    // React lets components throw any value. Normalize so fallbacks that
    // read `error.message` never render a blank diagnostic.
    return { error: toError(error) };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // Surface the failure to the JS console so the operator sees the stack;
    // production builds drop `console.*` via Vite's esbuild config so this
    // does not leak in shipped bundles.
    console.error('trogon-atlas studio: render error', error, info.componentStack);
  }

  reset = () => this.setState({ error: null });

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    if (this.props.fallback) return this.props.fallback({ error, reset: this.reset });
    return (
      <div role="alert" style={{ padding: 24, fontFamily: 'system-ui' }}>
        <h2 style={{ marginTop: 0 }}>Something went wrong rendering this view.</h2>
        <pre style={{ background: '#fef2f2', padding: 12, borderRadius: 6, overflow: 'auto' }}>{error.message}</pre>
        <button
          type="button"
          onClick={this.reset}
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
      </div>
    );
  }
}

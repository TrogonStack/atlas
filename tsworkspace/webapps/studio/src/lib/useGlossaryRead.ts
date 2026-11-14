import { useEffect, useState } from 'react';

export type GlossaryReadState<T> =
  | { status: 'idle' | 'loading'; key: string | null }
  | { status: 'ready'; key: string; value: T }
  | { status: 'failed'; key: string; error: string };

export function useGlossaryRead<T>(key: string | null, load: (signal: AbortSignal) => Promise<T>) {
  const [state, setState] = useState<GlossaryReadState<T>>({ status: 'idle', key: null });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (key === null) return;
    const controller = new AbortController();
    setState({ status: 'loading', key });
    load(controller.signal).then(
      (value) => {
        if (!controller.signal.aborted) setState({ status: 'ready', key, value });
      },
      (error: unknown) => {
        if (!controller.signal.aborted)
          setState({ status: 'failed', key, error: error instanceof Error ? error.message : String(error) });
      },
    );
    return () => controller.abort();
  }, [key, load, attempt]);
  const current: GlossaryReadState<T> =
    key === null ? { status: 'idle', key: null } : state.key === key ? state : { status: 'loading', key };
  return { state: current, retry: () => setAttempt((value) => value + 1) };
}

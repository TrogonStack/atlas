// The canvas's top-right corner, owned by one flex row instead of by whoever
// renders there last.
//
// ModelShell put the validator summary at `absolute right-2 top-2`, and every
// board that wanted a control put it at `absolute top-3 right-3`: two
// stacking contexts anchored to the same corner, so the sequence board's
// read-model filter rendered straight on top of the summary's counts. Boards
// now render through BoardTopRight into the single row below, where a new
// control pushes its neighbours aside instead of covering them.

import { createContext, useContext, useState } from 'react';
import { createPortal } from 'react-dom';

const TopRightHost = createContext<HTMLElement | null>(null);

export function BoardTopRightHost({ corner, children }: { corner: React.ReactNode; children: React.ReactNode }) {
  const [host, setHost] = useState<HTMLDivElement | null>(null);
  return (
    <TopRightHost.Provider value={host}>
      {/* Wraps rather than grows past the view switcher on the far left. */}
      <div
        ref={setHost}
        data-testid="board-top-right"
        className="absolute right-2 top-12 z-10 flex max-w-[75%] flex-wrap items-start justify-end gap-2 lg:top-2"
      >
        {/* Portaled controls append after this node, so the summary is ordered back to the corner. */}
        <div className="order-last">{corner}</div>
      </div>
      {children}
    </TopRightHost.Provider>
  );
}

export function BoardTopRight({ children }: { children: React.ReactNode }) {
  const host = useContext(TopRightHost);
  // A board rendered on its own (no shell) still needs the corner it used to own.
  if (!host) return <div className="absolute right-3 top-3 z-10 flex items-start gap-2">{children}</div>;
  return createPortal(children, host);
}

// Ambient access to the validator's findings for whatever model is on screen.
//
// A context rather than a prop because the consumers are board nodes, which
// @xyflow/react constructs from layout data. Threading findings there would
// mean stamping them onto StickyData, which would make layout.ts depend on
// validation and recompute the whole graph whenever a finding arrives.
// Layout stays a pure function of the model; findings arrive beside it.

import { createContext, useContext } from 'react';
import { EMPTY_ISSUE_INDEX, type IssueIndex } from '@/lib/issues';

// Defaulting to the empty index means a node rendered outside a provider
// reports no findings, never that the model is clean: EMPTY_ISSUE_INDEX
// carries no `error` and no counts, so nothing downstream can mistake it
// for a validated result.
const IssuesContext = createContext<IssueIndex>(EMPTY_ISSUE_INDEX);

export function IssuesProvider({ issues, children }: { issues: IssueIndex; children: React.ReactNode }) {
  return <IssuesContext.Provider value={issues}>{children}</IssuesContext.Provider>;
}

export function useIssues(): IssueIndex {
  return useContext(IssuesContext);
}

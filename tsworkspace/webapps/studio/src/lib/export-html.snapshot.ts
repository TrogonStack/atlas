// Stands in for lib/export-html in the snapshot runtime. A saved board does
// not export itself again, and leaving the real module out keeps its literal
// `atlas-snapshot` tag out of every saved file.
export function exportBoardHtml(): Promise<never> {
  return Promise.reject(new Error('a snapshot cannot export itself'));
}

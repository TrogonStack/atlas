/** Save `body` as a file named `fileName` through the browser's download flow. */
export function download(fileName: string, body: string, type: string): void {
  const url = URL.createObjectURL(new Blob([body], { type }));
  const a = document.createElement('a');
  a.href = url;
  a.download = fileName;
  document.body.appendChild(a);
  a.click();
  a.remove();
  // Revoke on the next tick: some browsers start the download after click returns.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

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

/**
 * Save exact `bytes` through the browser's download flow. The Blob is always typed as an opaque
 * octet stream, so declared media such as HTML or SVG is never interpreted by the browser.
 * Returns a function that revokes the object URL immediately; it also revokes on the next tick.
 */
export function downloadBytes(fileName: string, bytes: Uint8Array): () => void {
  const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'application/octet-stream' }));
  let revoked = false;
  const revoke = () => {
    if (revoked) return;
    revoked = true;
    URL.revokeObjectURL(url);
  };
  const a = document.createElement('a');
  a.href = url;
  a.download = fileName;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(revoke, 0);
  return revoke;
}

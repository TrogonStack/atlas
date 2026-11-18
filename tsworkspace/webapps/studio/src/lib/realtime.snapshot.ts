// Stands in for lib/realtime in the snapshot runtime. A snapshot never
// subscribes, so the NATS client would only add weight to every saved file.
export function watchEntities(): Promise<never> {
  return Promise.reject(new Error('a snapshot does not update'));
}

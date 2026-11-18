import { describe, expect, it } from 'vitest';
import { parseSnapshot, SNAPSHOT_VERSION, snapshotFileName, snapshotResponse, type StudioSnapshot } from './snapshot';

function snapshot(overrides: Partial<StudioSnapshot> = {}): StudioSnapshot {
  return {
    version: SNAPSHOT_VERSION,
    capturedAt: '2026-10-13T17:10:00.000Z',
    namespace: 'acme',
    slug: 'checkout',
    responses: { '/api/info': { serverVersion: '0.1.0' } },
    ...overrides,
  };
}

describe('parseSnapshot', () => {
  it('round-trips a valid snapshot', () => {
    const s = snapshot({ branch: 'alex/x' });
    expect(parseSnapshot(JSON.stringify(s))).toEqual(s);
  });

  it('rejects text that is not JSON', () => {
    expect(() => parseSnapshot('<html>')).toThrow('not valid JSON');
  });

  it('rejects a version it does not know', () => {
    expect(() => parseSnapshot(JSON.stringify({ ...snapshot(), version: 2 }))).toThrow(
      'unsupported snapshot version: 2',
    );
  });

  it.each(['capturedAt', 'namespace', 'slug'])('rejects a snapshot without %s', (field) => {
    const s: Record<string, unknown> = { ...snapshot() };
    delete s[field];
    expect(() => parseSnapshot(JSON.stringify(s))).toThrow(`missing ${field}`);
  });

  it('rejects a snapshot without responses', () => {
    expect(() => parseSnapshot(JSON.stringify({ ...snapshot(), responses: [] }))).toThrow('missing responses');
  });

  it('round-trips the manifests a snapshot carries', () => {
    const s = snapshot({ manifests: { model: 'kind: EventModel\n', external: '' } });
    expect(parseSnapshot(JSON.stringify(s))).toEqual(s);
  });

  it.each([
    ['a string', 'kind: EventModel'],
    ['no external part', { model: 'kind: EventModel' }],
    ['a non-string model', { model: 1, external: '' }],
  ])('rejects manifests that are %s', (_, manifests) => {
    expect(() => parseSnapshot(JSON.stringify({ ...snapshot(), manifests }))).toThrow('snapshot manifests');
  });
});

describe('snapshotResponse', () => {
  it('returns the recorded body for a path', () => {
    expect(snapshotResponse(snapshot(), '/api/info')).toEqual({ serverVersion: '0.1.0' });
  });

  it('throws for a path the snapshot does not hold, instead of returning nothing', () => {
    expect(() => snapshotResponse(snapshot(), '/api/branches')).toThrow('not in this snapshot: /api/branches');
  });

  it('does not treat inherited object keys as recorded paths', () => {
    expect(() => snapshotResponse(snapshot(), 'toString')).toThrow('not in this snapshot');
  });
});

describe('snapshotFileName', () => {
  it('names a baseline snapshot by slug and UTC time', () => {
    expect(snapshotFileName(snapshot())).toBe('checkout-20261013-1710.html');
  });

  it('adds the branch and makes it safe for a file name', () => {
    expect(snapshotFileName(snapshot({ branch: 'alex/new flow' }))).toBe('checkout-alex-new-flow-20261013-1710.html');
  });

  it('takes another extension for the files saved next to the board', () => {
    expect(snapshotFileName(snapshot(), '.external.yaml')).toBe('checkout-20261013-1710.external.yaml');
  });

  it('still produces a name when the time cannot be parsed', () => {
    expect(snapshotFileName(snapshot({ capturedAt: 'later' }))).toBe('checkout-snapshot.html');
  });
});

import { describe, expect, it } from 'vitest';
import { buildSnapshotHtml } from './export-html';
import { parseSnapshot, SNAPSHOT_VERSION, type StudioSnapshot } from './snapshot';

const snapshot: StudioSnapshot = {
  version: SNAPSHOT_VERSION,
  capturedAt: '2026-10-13T17:10:00.000Z',
  namespace: 'acme',
  slug: 'checkout',
  responses: {
    '/api/event-model/acme/checkout': {
      entities: [{ event: { doc: 'ends with </script><script>alert(1)</script>' } }],
    },
  },
};

function build(overrides: Partial<Parameters<typeof buildSnapshotHtml>[0]> = {}): string {
  return buildSnapshotHtml({ js: 'console.log(1)', css: 'body{}', snapshot, title: 'Checkout', ...overrides });
}

// What the browser would hand to JSON.parse: the text up to the first closing tag.
function embeddedData(html: string): string {
  const start = html.indexOf('<script id="atlas-snapshot" type="application/json">');
  const open = html.indexOf('>', start) + 1;
  return html.slice(open, html.indexOf('</script', open));
}

describe('buildSnapshotHtml', () => {
  it('embeds a snapshot that parses back unchanged, even when model text contains </script>', () => {
    const html = build();
    expect(parseSnapshot(embeddedData(html))).toEqual(snapshot);
  });

  it('keeps a closing script tag in the runtime from ending the inline script', () => {
    const html = build({ js: 'const s = "</script>";' });
    expect(html).toContain('<script type="module">const s = "\\x3C/script>";</script>');
  });

  it('keeps an HTML comment opener in the runtime from changing where the script ends', () => {
    expect(build({ js: 'const s = "<!--";' })).toContain('const s = "\\x3C!--";');
  });

  it('escapes the runtime in a form JS still reads as the original text', () => {
    const js = 'globalThis.out = ["</script>", `<!--`, /<\\/script/.source];';
    const html = build({ js });
    const inlined = html.slice(html.indexOf('<script type="module">') + 22, html.lastIndexOf('</script>'));
    const scope: { out?: unknown } = {};
    new Function('globalThis', inlined)(scope);
    expect(scope.out).toEqual(['</script>', '<!--', '<\\/script']);
  });

  it('keeps a closing style tag in the stylesheet from ending the inline style', () => {
    expect(build({ css: 'a{content:"</style>"}' })).toContain('<style>a{content:"<\\/style>"}</style>');
  });

  it('escapes the title', () => {
    expect(build({ title: 'A <b> & "c"' })).toContain('<title>A &lt;b&gt; &amp; &quot;c&quot; (snapshot)</title>');
  });

  it('puts the data before the runtime that reads it', () => {
    const html = build();
    expect(html.indexOf('id="atlas-snapshot"')).toBeLessThan(html.indexOf('<script type="module">'));
  });
});

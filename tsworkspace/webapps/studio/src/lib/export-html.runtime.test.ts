import { beforeEach, describe, expect, it, vi } from 'vitest';

const { apiMock, downloadMock } = vi.hoisted(() => ({
  apiMock: {
    info: vi.fn(),
    eventModel: vi.fn(),
    validate: vi.fn(),
    manifests: vi.fn(),
  },
  downloadMock: vi.fn(),
}));

vi.mock('@/lib/api', () => ({
  api: apiMock,
  boardPaths: {
    info: () => '/api/info',
    eventModel: (namespace: string, slug: string) => `/api/event-model/${namespace}/${slug}`,
    validate: ({ namespace, slug }: { namespace: string; slug: string }) => `/api/validate/${namespace}/${slug}`,
  },
}));
vi.mock('@/lib/download', () => ({ download: downloadMock }));

function response(status: number, contentType: string, body = '') {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: { get: (name: string) => (name.toLowerCase() === 'content-type' ? contentType : null) },
    text: async () => body,
    json: async () => JSON.parse(body),
  };
}

async function loadExportBoardHtml() {
  vi.resetModules();
  return (await import('./export-html')).exportBoardHtml;
}

describe('exportBoardHtml runtime failures', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    apiMock.info.mockResolvedValue({});
    apiMock.eventModel.mockResolvedValue({});
    apiMock.validate.mockResolvedValue({});
    apiMock.manifests.mockResolvedValue(undefined);
  });

  it('reports the HTTP status when the dev-server runtime build fails', async () => {
    const exportBoardHtml = await loadExportBoardHtml();
    vi.stubGlobal(
      'fetch',
      vi.fn((path: string) =>
        Promise.resolve(
          path === '/snapshot/runtime' || path === '/snapshot/runtime.js'
            ? response(500, 'text/html', '<!doctype html>')
            : response(200, 'text/css', 'body{}'),
        ),
      ),
    );

    await expect(exportBoardHtml({ namespace: 'acme', slug: 'checkout', title: 'Checkout' })).rejects.toThrow(
      'HTTP 500',
    );
  });

  it('requests JavaScript and CSS as one runtime generation in dev', async () => {
    const exportBoardHtml = await loadExportBoardHtml();
    vi.stubGlobal(
      'fetch',
      vi.fn(() =>
        Promise.resolve(
          response(200, 'application/json', JSON.stringify({ js: 'console.log(1)', css: 'body{}' })),
        ),
      ),
    );

    await exportBoardHtml({ namespace: 'acme', slug: 'checkout', title: 'Checkout' });

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(fetch).toHaveBeenCalledWith('/snapshot/runtime');
  });

  it('refreshes the runtime after a dev-server source change', async () => {
    const exportBoardHtml = await loadExportBoardHtml();
    const fetchMock = vi.fn();
    fetchMock.mockImplementation(() => {
      const generation = fetchMock.mock.calls.length === 0 ? 'runtime-a' : 'runtime-b';
      return Promise.resolve(response(200, 'application/json', JSON.stringify({ js: generation, css: generation })));
    });
    vi.stubGlobal('fetch', fetchMock);

    await exportBoardHtml({ namespace: 'acme', slug: 'checkout', title: 'Checkout' });
    await exportBoardHtml({ namespace: 'acme', slug: 'checkout', title: 'Checkout' });

    expect(downloadMock.mock.calls[1]?.[1]).toContain('<script type="module">runtime-b</script>');
  });
});

import { beforeEach, describe, expect, it, vi } from 'vitest';

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    info: vi.fn(),
    eventModel: vi.fn(),
    validate: vi.fn(),
    manifests: vi.fn(),
  },
}));

vi.mock('@/lib/api', () => ({
  api: apiMock,
  boardPaths: {
    info: () => '/api/info',
    eventModel: (namespace: string, slug: string) => `/api/event-model/${namespace}/${slug}`,
    validate: ({ namespace, slug }: { namespace: string; slug: string }) => `/api/validate/${namespace}/${slug}`,
  },
}));
vi.mock('@/lib/download', () => ({ download: vi.fn() }));

import { exportBoardHtml } from './export-html';

function response(status: number, contentType: string, body = '') {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: { get: (name: string) => (name.toLowerCase() === 'content-type' ? contentType : null) },
    text: async () => body,
  };
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
    vi.stubGlobal(
      'fetch',
      vi.fn((path: string) =>
        Promise.resolve(
          path === '/snapshot/runtime.js' ? response(500, 'text/html', '<!doctype html>') : response(200, 'text/css', 'body{}'),
        ),
      ),
    );

    await expect(exportBoardHtml({ namespace: 'acme', slug: 'checkout', title: 'Checkout' })).rejects.toThrow(
      'HTTP 500',
    );
  });
});

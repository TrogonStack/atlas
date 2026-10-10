import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  testMatch: '**/*.spec.ts',
  timeout: 30_000,
  retries: 0,
  workers: 1,
  reporter: 'list',
  use: {
    baseURL: 'http://localhost:5179',
    headless: true,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: {
    // Boots the real bridge (server/index.mjs) and the real Vite dev
    // server, both pointed at a hermetic in-process fake gRPC backend --
    // see e2e/fixtures/run-e2e-stack.mjs and e2e/README.md for why this
    // topology (vs. mocking fetch/route in the browser) is the point of
    // this suite.
    command: 'mise exec -- node e2e/fixtures/run-e2e-stack.mjs',
    url: 'http://localhost:5179',
    reuseExistingServer: false,
    timeout: 30_000,
  },
});

import path from 'node:path';
import { defineConfig, devices } from '@playwright/test';

if (!process.env.ATLAS_NATIVE_EVIDENCE_DIR)
  throw new Error('Set ATLAS_NATIVE_EVIDENCE_DIR under the repository .trogonai directory');
export default defineConfig({
  testDir: './e2e',
  testMatch: '**/*.native.spec.ts',
  timeout: 600_000,
  workers: 1,
  retries: 0,
  outputDir: path.join(process.env.ATLAS_NATIVE_EVIDENCE_DIR, 'playwright'),
  reporter: [['list']],
  use: { ...devices['Desktop Chrome'], headless: true, trace: 'retain-on-failure' },
});

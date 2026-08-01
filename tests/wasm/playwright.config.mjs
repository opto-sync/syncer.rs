// @ts-check
/**
 * Playwright configuration for the wasm browser conformance suite.
 *
 * The suite needs a real HTTP origin (see serve.mjs), so the static server is
 * started as a `webServer` and torn down with the run.
 */

import { defineConfig, devices } from '@playwright/test';

const PORT = Number(process.env.PORT ?? 4173);

export default defineConfig({
  testDir: '.',
  testMatch: /.*\.spec\.mjs$/,
  // The corpus is deterministic and pure; a retry would only mask flakiness in
  // the harness itself.
  retries: 0,
  forbidOnly: !!process.env.CI,
  workers: 1,
  timeout: 60_000,
  reporter: process.env.CI ? [['github'], ['list']] : [['list']],
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: {
    command: `node ${new URL('./serve.mjs', import.meta.url).pathname}`,
    env: { PORT: String(PORT) },
    url: `http://127.0.0.1:${PORT}/tests/wasm/harness.html`,
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
    stdout: 'pipe',
    stderr: 'pipe',
  },
});

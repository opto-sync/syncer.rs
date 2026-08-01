// @ts-check
/**
 * Executes the wasm conformance corpus inside real Chromium.
 *
 * The page imports `cases.mjs` directly, so the browser runs the same corpus
 * source as `run-node.mjs` rather than a serialized copy. Each case is
 * reported individually via soft assertions, so one regression does not mask
 * the rest.
 */

import { test, expect } from '@playwright/test';

import { cases } from './cases.mjs';

/** @type {{name: string, ok: boolean, detail: string}[] | null} */
let results = null;

test.beforeAll(async ({ browser }) => {
  const page = await browser.newPage();

  /** @type {string[]} */
  const consoleErrors = [];
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });
  page.on('pageerror', (error) => consoleErrors.push(String(error)));

  await page.goto('/tests/wasm/harness.html');
  await page.waitForFunction(() => window.__syncerDone === true, null, { timeout: 60_000 });

  const harnessError = await page.evaluate(() => window.__syncerError);
  if (harnessError) {
    throw new Error(`wasm harness failed to initialize in Chromium:\n${harnessError}`);
  }

  results = await page.evaluate(() => window.__syncerResults);

  // An uncaught error in the glue can still leave a usable results array;
  // fail loudly rather than reporting a green run over a broken page.
  expect(consoleErrors, 'browser console must be clean').toEqual([]);

  await page.close();
});

test('every corpus case runs in Chromium', () => {
  expect(results, 'harness produced results').not.toBeNull();
  expect(results).toHaveLength(cases.length);
});

// One test per case so the Playwright report names the exact regression.
for (const [index, testCase] of cases.entries()) {
  test(`chromium: ${testCase.name}`, () => {
    const result = results?.[index];
    expect(result, `case ${index} was executed`).toBeTruthy();
    expect(result?.name).toBe(testCase.name);
    expect(result?.ok, result?.detail).toBe(true);
  });
}

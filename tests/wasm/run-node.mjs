#!/usr/bin/env node
/**
 * Executes the shared wasm corpus under Node.
 *
 * Uses the `--target web` build (not `nodejs`) so this runner and the Chromium
 * spec load byte-identical artifacts; a Node-only regression in the browser
 * glue would otherwise go unnoticed until a consumer hit it.
 *
 *   make pkg-web && node tests/wasm/run-node.mjs
 *
 * Set SYNCER_WASM_PKG to point at a different wasm-pack output directory.
 */

import { readFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';

import { cases, runCases } from './cases.mjs';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const pkgDir = path.resolve(repoRoot, process.env.SYNCER_WASM_PKG ?? 'pkg-web');
const glue = path.join(pkgDir, 'syncer_rs.js');
const binary = path.join(pkgDir, 'syncer_rs_bg.wasm');

if (!existsSync(glue) || !existsSync(binary)) {
  console.error(
    `No wasm build at ${pkgDir}.\n` +
      `Build it first:  make pkg-web\n` +
      `(or set SYNCER_WASM_PKG to an existing wasm-pack --target web output)`,
  );
  process.exit(2);
}

const module = await import(pathToFileURL(glue).href);
await module.default({ module_or_path: await readFile(binary) });

const failures = runCases({
  mergeJson: module.mergeJson,
  mergeJsonWithOptions: module.mergeJsonWithOptions,
});

if (failures.length > 0) {
  console.error(`\n${failures.length} of ${cases.length} wasm cases failed under Node:\n`);
  for (const failure of failures) {
    console.error(`  ✗ ${failure}`);
  }
  process.exit(1);
}

console.log(`✓ ${cases.length} wasm cases passed under Node (${path.relative(repoRoot, pkgDir)})`);

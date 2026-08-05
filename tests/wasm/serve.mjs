#!/usr/bin/env node
/**
 * Dependency-free static server for the browser harness.
 *
 * Serves the repository root over HTTP so the page can fetch
 * `/pkg-web/syncer_rs_bg.wasm` the way a real deployment would. Serving
 * `application/wasm` correctly matters: `WebAssembly.instantiateStreaming`
 * rejects any other content type, and a file:// page cannot fetch the module
 * at all — which is exactly why this test needs a server rather than
 * `page.setContent`.
 *
 * Listens on 127.0.0.1 only. PORT selects the port (0 picks a free one and the
 * chosen port is printed as `listening <port>`).
 */

import http from 'node:http';
import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

const CONTENT_TYPES = new Map([
  ['.html', 'text/html; charset=utf-8'],
  ['.js', 'text/javascript; charset=utf-8'],
  ['.mjs', 'text/javascript; charset=utf-8'],
  ['.json', 'application/json; charset=utf-8'],
  ['.wasm', 'application/wasm'],
  ['.ts', 'text/plain; charset=utf-8'],
]);

const server = http.createServer(async (request, response) => {
  const requested = decodeURIComponent(new URL(request.url, 'http://127.0.0.1').pathname);
  const resolved = path.resolve(repoRoot, `.${requested}`);

  // Refuse anything that escapes the repository root.
  if (resolved !== repoRoot && !resolved.startsWith(repoRoot + path.sep)) {
    response.writeHead(403).end('forbidden');
    return;
  }

  try {
    const info = await stat(resolved);
    if (!info.isFile()) {
      response.writeHead(404).end('not found');
      return;
    }
    response.writeHead(200, {
      'content-type': CONTENT_TYPES.get(path.extname(resolved)) ?? 'application/octet-stream',
      'content-length': info.size,
      'cache-control': 'no-store',
    });
    createReadStream(resolved).pipe(response);
  } catch {
    response.writeHead(404).end('not found');
  }
});

server.listen(Number(process.env.PORT ?? 0), '127.0.0.1', () => {
  console.log(`listening ${server.address().port}`);
});

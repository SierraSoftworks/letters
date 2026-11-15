// Serves `dist/` on localhost, for the harness page and the browser tests.
//
//   node serve.mjs [port]

import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const DIST = fileURLToPath(new URL("dist", import.meta.url));
// A browser compiles wasm as it downloads only when it is served as wasm.
const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".ico": "image/x-icon",
  ".js": "text/javascript",
  ".ttf": "font/ttf",
  ".txt": "text/plain; charset=utf-8",
  ".wasm": "application/wasm",
};
const port = Number(process.argv[2] ?? 8080);

createServer(async (request, response) => {
  const path = normalize(new URL(request.url, "http://localhost").pathname);
  const file = join(DIST, path.endsWith("/") ? `${path}index.html` : path);
  try {
    const body = await readFile(file);
    response.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" });
    response.end(body);
  } catch {
    response.writeHead(404).end("Not found");
  }
}).listen(port, "127.0.0.1", () => console.log(`Letters is at http://localhost:${port}/`));

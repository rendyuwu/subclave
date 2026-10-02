import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// The fixture origin Playwright serves. `playwright.config.ts` starts this with
// `webServer` and passes the port through E2E_PORT.
const root = resolve(fileURLToPath(new URL("../fixtures", import.meta.url)));
const port = Number(process.env.E2E_PORT ?? 41731);

const TYPES = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
};

const server = createServer(async (request, response) => {
  const pathname = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
  const target = normalize(join(root, pathname === "/" ? "/index.html" : pathname));
  if (!target.startsWith(root)) {
    response.writeHead(403).end("forbidden");
    return;
  }
  try {
    const body = await readFile(target);
    response.writeHead(200, {
      "cache-control": "no-store",
      "content-type": TYPES[extname(target)] ?? "application/octet-stream",
    });
    response.end(body);
  } catch {
    response.writeHead(404).end("not found");
  }
});

server.listen(port, "127.0.0.1", () => {
  console.log(`fixtures on http://127.0.0.1:${port}`);
});

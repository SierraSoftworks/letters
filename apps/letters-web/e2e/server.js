// A server behind the page, for the tests: the page it sends for a document it keeps, and its answers.

import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { crc32, deflateSync } from "node:zlib";

const ID = "0190f3a2-7c1e-7b4a-9c55-3f2a1b0c9d8e";

// What the server sends its pages under: nothing inline runs, and nothing is fetched from elsewhere.
const POLICY =
  "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'unsafe-inline'; img-src 'self' blob: data:; " +
  "font-src 'self'; connect-src 'self' ws://localhost:*; frame-src blob:; manifest-src 'self'; base-uri 'self'; form-action 'none'; frame-ancestors 'none'";

/** What a server says of a document its person may edit, as `letters_api::Opened`. */
export const opened = (over = {}) => ({
  id: ID,
  created: "2026-11-15T10:00:00Z",
  name: "Minutes",
  role: "editor",
  author: "Ada Lovelace",
  socket: `/api/v1/documents/${ID}/sync`,
  trust: "trusted",
  allows: { download: true, copy: true, print: true },
  ...over,
});

/** A PNG of one colour, and the name a document has for it: `assets/` and the hash of its bytes. */
export const picture = ([red, green, blue], width = 64, height = 48) => {
  const chunk = (kind, data) => {
    const body = Buffer.concat([Buffer.from(kind), data]);
    const [length, sum] = [Buffer.alloc(4), Buffer.alloc(4)];
    length.writeUInt32BE(data.length);
    sum.writeUInt32BE(crc32(body));
    return Buffer.concat([length, body, sum]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8);
  const line = Buffer.from([0, ...Array(width).fill([red, green, blue]).flat()]);
  const lines = deflateSync(Buffer.concat(Array(height).fill(line)));
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const bytes = Buffer.concat([signature, chunk("IHDR", header), chunk("IDAT", lines), chunk("IEND", Buffer.alloc(0))]);
  return { bytes, name: `assets/${createHash("sha256").update(bytes).digest("hex")}.png` };
};

/** A document that shows these figures under a heading: each is its `asset` or its `src`, with a size. */
export const album = (figures) => ({
  format: 1,
  language: "en",
  blocks: [
    { id: "h", type: "heading", level: 1, content: [{ text: "Album" }] },
    ...figures.map((figure, at) => ({ id: `f${at}`, type: "figure", alt: "", width: 64, height: 48, ...figure })),
    { id: "p", type: "paragraph", content: [] },
  ],
});

const pages = new Map();
const cwd = fileURLToPath(new URL("..", import.meta.url));

/** Writes each of `values` that is given into a file of its own, as JSON, and gives the files' paths. */
const written = (...values) => {
  const folder = mkdtempSync(join(tmpdir(), "letters-served-"));
  return values.filter(Boolean).map((json, at) => {
    writeFileSync(join(folder, `${at}.json`), JSON.stringify(json));
    return join(folder, `${at}.json`);
  });
};

/** The page the server sends, written by the call a server makes: `letters_web::page::served`. */
const composed = (said, document) => {
  const key = JSON.stringify([said, document]);
  if (!pages.has(key)) {
    const run = ["run", "--quiet", "-p", "letters-web", "--example", "page", "--", "--served", "/", ...written(said, document)];
    pages.set(key, execFileSync("cargo", run, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }));
  }
  return pages.get(key);
};

const ROLES = ["reader", "commenter", "editor"];

/**
 * The room a server keeps for the document `said` is about, which the sockets of its pages lead to:
 * `letters_sync::Room` itself, run by the `room` example. `document` is its `document.json`, the welcome
 * document if none is given.
 */
export const room = (said = opened(), document = undefined) => {
  const run = ["run", "--quiet", "-p", "letters-web", "--example", "room", "--", ...written(said, document)];
  const child = spawn("cargo", run, { cwd, stdio: ["pipe", "pipe", "inherit"] });
  const [sockets, asked] = [new Map(), []];
  let [connections, down, read] = [0, false, Buffer.alloc(0)];
  // A record each way: a kind, a connection and a length, then that many bytes.
  const send = (kind, connection, bytes = Buffer.alloc(0)) => {
    const head = Buffer.alloc(9);
    head.writeUInt8(kind);
    head.writeUInt32LE(connection, 1);
    head.writeUInt32LE(bytes.length, 5);
    child.stdin.write(Buffer.concat([head, bytes]));
  };
  child.stdout.on("data", (chunk) => {
    read = Buffer.concat([read, chunk]);
    while (read.length >= 9 && read.length >= 9 + read.readUInt32LE(5)) {
      const [kind, connection, bytes] = [read.readUInt8(), read.readUInt32LE(1), read.subarray(9, 9 + read.readUInt32LE(5))];
      read = read.subarray(9 + bytes.length);
      if (kind === 0) sockets.get(connection)?.send(Buffer.from(bytes));
      if (kind === 1) {
        const socket = sockets.get(connection);
        left(connection);
        socket?.close();
      }
      if (kind === 2) asked.shift()?.(JSON.parse(bytes.toString()));
    }
  });
  const left = (connection) => sockets.delete(connection) && send(2, connection);
  return {
    /** Has the page's socket lead here, for a person with this role and name. */
    join: (page, { role = "editor", name = "Ada Lovelace" } = {}) =>
      page.routeWebSocket(/\/sync$/, (socket) => {
        if (down) return socket.close();
        const connection = ++connections;
        sockets.set(connection, socket);
        send(0, connection, Buffer.concat([Buffer.from([ROLES.indexOf(role)]), Buffer.from(name)]));
        socket.onMessage((frame) => sockets.has(connection) && send(1, connection, Buffer.from(frame)));
        socket.onClose(() => left(connection));
      }),
    /** The room's `document.json` as it stands. */
    document: () => new Promise((done) => (asked.push(done), send(3, 0))),
    /** The text of the room's document, a block each. */
    texts: async function () {
      return (await this.document()).blocks.map((block) => (block.content ?? []).map((run) => run.text).join(""));
    },
    /** Cuts every socket, and lets none through until `mend`. */
    cut: () => {
      down = true;
      for (const [connection, socket] of [...sockets]) {
        left(connection);
        socket.close();
      }
    },
    mend: () => (down = false),
    /** Whether no socket is let through. */
    cutOff: () => down,
    close: () => child.stdin.end(),
  };
};

/**
 * Opens the server's page for the document `said` is about, and waits for the editor as `ready` does.
 * `answer(method, path, body)` is what the server answers a request with, as `{ status, body, type, headers }` or a
 * promise of it: nothing is there otherwise. `document` is its `document.json`, the welcome document if none is given. The page's
 * socket leads to `host`, a room of that document that is the page's own unless one is given. Resolves to the
 * requests the page made of the server, as they come, once the page is in session if the room lets it be.
 */
export const serve = async (page, said, answer = () => undefined, document = undefined, host = undefined) => {
  const asked = [];
  if (!host) {
    host = room(said, document);
    page.once("close", host.close);
  }
  await host.join(page, { role: said.role, name: said.author });
  // What the policy stopped, for a test to find nothing in.
  await page.addInitScript(() => {
    window.blocked = [];
    document.addEventListener("securitypolicyviolation", (event) =>
      window.blocked.push(`${event.violatedDirective} ${event.blockedURI}`),
    );
  });
  await page.route(`**/d/${said.id}`, (route) =>
    route.fulfill({ contentType: "text/html; charset=utf-8", headers: { "content-security-policy": POLICY }, body: composed(said, document) }),
  );
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    // A page looks up what the person set for themselves, the sources they trust, the server's dictionaries
    // and what the document is called by itself, and says where its person read up to: none of it is what a
    // test asked of it.
    const looked = ["/api/v1/settings", "/api/v1/settings/sources", "/api/v1/dictionaries", `/api/v1/documents/${said.id}`];
    const own = request.method() === "GET" ? looked : [`/api/v1/documents/${said.id}/seen`];
    if (!own.includes(path)) {
      asked.push({ method: request.method(), path, body: request.postDataBuffer(), type: await request.headerValue("content-type") });
    }
    const nothing = { status: 404, body: { code: "not_found", message: "Nothing is there." } };
    const { status = 200, body = "", type = "application/json", headers } = (await answer(request.method(), path, request.postData())) ?? nothing;
    const sent = typeof body === "string" || Buffer.isBuffer(body) ? body : JSON.stringify(body);
    await route.fulfill({ status, contentType: type, headers, body: sent });
  });
  await page.goto(`/d/${said.id}`);
  await page.waitForFunction(() => window.letters?.frames > 0 && window.letters.fonts && window.letters.spelling);
  if (!host.cutOff()) await page.waitForFunction(() => window.letters.live);
  return asked;
};

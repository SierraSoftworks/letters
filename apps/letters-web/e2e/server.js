// A server behind the page, for the tests: the page it sends for a document it keeps, and its answers.

import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const ID = "0190f3a2-7c1e-7b4a-9c55-3f2a1b0c9d8e";

// What the server sends its pages under: nothing inline runs, and nothing is fetched from elsewhere.
const POLICY =
  "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'unsafe-inline'; img-src 'self' blob: data:; " +
  "font-src 'self'; connect-src 'self'; frame-src blob:; base-uri 'self'; form-action 'none'; frame-ancestors 'none'";

/** What a server says of a document its person may edit, as `letters_api::Opened`. */
export const opened = (over = {}) => ({
  id: ID,
  created: "2026-11-15T10:00:00Z",
  name: "Minutes",
  role: "editor",
  author: "Ada Lovelace",
  socket: `/api/v1/documents/${ID}/socket`,
  trust: "trusted",
  allows: { download: true, copy: true, print: true },
  ...over,
});

const pages = new Map();

/** The page the server sends, written by the call a server makes: `letters_web::page::served`. */
const composed = (said) => {
  const key = JSON.stringify(said);
  if (!pages.has(key)) {
    const file = join(mkdtempSync(join(tmpdir(), "letters-served-")), "opened.json");
    writeFileSync(file, key);
    const run = ["run", "--quiet", "-p", "letters-web", "--example", "page", "--", "--served", "/", file];
    const cwd = fileURLToPath(new URL("..", import.meta.url));
    pages.set(key, execFileSync("cargo", run, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }));
  }
  return pages.get(key);
};

/**
 * Opens the server's page for the document `said` is about, and waits for the editor as `ready` does.
 * `answer(method, path)` is what the server answers a request with, as `{ status, body, type }`: nothing is
 * there otherwise. Resolves to the requests the page made of the server, as they come.
 */
export const serve = async (page, said, answer = () => undefined) => {
  const asked = [];
  // What the policy stopped, for a test to find nothing in.
  await page.addInitScript(() => {
    window.blocked = [];
    document.addEventListener("securitypolicyviolation", (event) =>
      window.blocked.push(`${event.violatedDirective} ${event.blockedURI}`),
    );
  });
  await page.route(`**/d/${said.id}`, (route) =>
    route.fulfill({ contentType: "text/html; charset=utf-8", headers: { "content-security-policy": POLICY }, body: composed(said) }),
  );
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    asked.push({ method: request.method(), path, body: request.postData() });
    const nothing = { status: 404, body: { code: "not_found", message: "Nothing is there." } };
    const { status = 200, body = "", type = "application/json" } = answer(request.method(), path) ?? nothing;
    const sent = typeof body === "string" || Buffer.isBuffer(body) ? body : JSON.stringify(body);
    await route.fulfill({ status, contentType: type, body: sent });
  });
  await page.goto(`/d/${said.id}`);
  await page.waitForFunction(() => window.letters?.frames > 0 && window.letters.fonts && window.letters.spelling);
  return asked;
};

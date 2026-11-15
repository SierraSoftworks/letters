// The page as letters-server itself sends it: browsers and a device that holds a copy, in one document.
//
// The server has the editor only when it is built with it, so this runs when the build is told where:
// `./build.sh && LETTERS_WEB_DIST=$PWD/dist npx playwright test e2e/server.spec.js --project chromium`.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import { expect, test } from "@playwright/test";

import { rested, texts } from "./page.js";

test.skip(!process.env.LETTERS_WEB_DIST, "letters-server is built with the editor when LETTERS_WEB_DIST names dist/");
// The first run builds the server.
test.setTimeout(20 * 60_000);

/** Starts the server over a folder of its own, with its people signed in: see `examples/server.rs`. */
const served = async () => {
  const folder = mkdtempSync(join(tmpdir(), "letters-server-"));
  const run = ["run", "--quiet", "--release", "-p", "letters-web", "--example", "server", "--", folder];
  const child = spawn("cargo", run, { cwd: fileURLToPath(new URL("..", import.meta.url)), stdio: ["pipe", "pipe", "inherit"] });
  const [said, waiting] = [[], []];
  createInterface({ input: child.stdout }).on("line", (line) => (waiting.shift() ?? ((value) => said.push(value)))(JSON.parse(line)));
  const next = () => (said.length ? Promise.resolve(said.shift()) : new Promise((done) => waiting.push(done)));
  const { url, cookies } = await next();
  return {
    folder,
    url,
    /** A browser that `who` is signed in with. */
    browser: async (browser, who) => {
      const context = await browser.newContext({ baseURL: url });
      await context.addCookies([{ name: "letters_session", value: cookies[who], url }]);
      // What the server's policy stopped, for the test to find nothing in.
      await context.addInitScript(() => {
        window.blocked = [];
        document.addEventListener("securitypolicyviolation", (event) => window.blocked.push(`${event.violatedDirective} ${event.blockedURI}`));
      });
      return context;
    },
    /** Has the device do something: `join <id>` or `write <text>`. */
    device: (line) => child.stdin.write(`${line}\n`),
    /** The text of the device's copy, a block each, once what is on its way has arrived. */
    texts: () => (child.stdin.write("texts\n"), next()),
    stop: () => child.stdin.end(),
  };
};

/** Opens the document in the editor and waits until the page is in session. */
const open = async (context, id) => {
  const page = await context.newPage();
  await page.goto(`/d/${id}`);
  await page.waitForFunction(() => window.letters?.frames > 0 && window.letters.live, undefined, { timeout: 60_000 });
  return page;
};

const shown = async (page) => (await texts(page)).join("\n");
const tools = (page) => page.getByRole("toolbar", { name: "Document" });

test("two browsers and a device write in one document that the server keeps", async ({ browser }) => {
  const server = await served();
  try {
    const [ada, grace] = [await server.browser(browser, "ada"), await server.browser(browser, "grace")];
    // A document in the space Ada's group edits and Grace's comments in.
    const made = await ada.request.post("/api/v1/spaces/family/documents", { data: { name: "Minutes" }, headers: { origin: server.url } });
    expect(made.ok()).toBe(true);
    const { id } = await made.json();

    const writer = await open(ada, id);
    await expect(writer).toHaveTitle("Minutes");
    await writer.keyboard.type("From Ada.");
    await expect(tools(writer).getByText("Saved", { exact: true })).toBeAttached();

    // Someone who comments reads what was written, and the bar says what they may do.
    const reviewer = await open(grace, id);
    await expect.poll(() => shown(reviewer)).toContain("From Ada.");
    await expect(tools(reviewer).getByText("Commenting", { exact: true })).toBeAttached();
    await expect(writer.getByLabel("Grace Hopper")).toBeAttached();

    // A device opens it as the desktop app does, with a copy of its own, and writes too.
    server.device(`join ${id}`);
    expect((await server.texts()).join("\n")).toContain("From Ada.");
    server.device("write From the desk. ");
    await expect.poll(() => shown(writer)).toContain("From the desk. ");
    await expect.poll(() => shown(reviewer)).toContain("From the desk. ");
    await writer.bringToFront();
    await writer.keyboard.type(" And again.");
    await expect.poll(async () => (await server.texts()).join("\n")).toContain("From Ada. And again.");

    // The name is the file's, which the server renames.
    await writer.keyboard.press("ControlOrMeta+Shift+p");
    await writer.keyboard.type("Rename or move");
    await rested(writer);
    await writer.keyboard.press("Enter");
    await writer.keyboard.press("ControlOrMeta+a");
    await writer.keyboard.type("Agenda");
    await writer.keyboard.press("Enter");
    await expect(writer).toHaveTitle("Agenda");
    await expect.poll(() => existsSync(join(server.folder, "family", "Agenda.let"))).toBe(true);

    // What the person sets is kept for them by the server.
    await writer.keyboard.press("ControlOrMeta+,");
    const dark = await writer.getByRole("button", { name: "Dark", exact: true }).boundingBox();
    await writer.mouse.click(dark.x + dark.width / 2, dark.y + dark.height / 2);
    await expect.poll(async () => (await (await ada.request.get("/api/v1/settings")).json()).appearance).toBe("dark");

    for (const page of [writer, reviewer]) {
      expect(await page.evaluate(() => window.blocked)).toEqual([]);
    }
  } finally {
    server.stop();
  }
});

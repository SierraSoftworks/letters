// The page as letters-server itself sends it: browsers and a device that holds a copy, in one document.
//
// The server has the editor only when it is built with it, so this runs when the build is told where:
// `./build.sh && LETTERS_WEB_DIST=$PWD/dist npx playwright test e2e/server.spec.js --project chromium`.

import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import { expect, test } from "@playwright/test";

import { learn, rested, texts } from "./page.js";

test.skip(!process.env.LETTERS_WEB_DIST, "letters-server is built with the editor when LETTERS_WEB_DIST names dist/");
// The server is built before the tests run (`built.js`), and cargo still looks whether it is.
test.setTimeout(2 * 60_000);

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

/** A browser the server has never seen. */
const stranger = async (browser, server) => {
  const guest = await browser.newContext({ baseURL: server.url });
  await guest.addInitScript(() => {
    window.blocked = [];
    document.addEventListener("securitypolicyviolation", (event) => window.blocked.push(`${event.violatedDirective} ${event.blockedURI}`));
  });
  return guest.newPage();
};

test("a link's visitor, signed in nowhere, gets the page to read as a reader and the editor within the link's limits to comment", async ({ browser }) => {
  const server = await served();
  try {
    const ada = await server.browser(browser, "ada");
    const post = (path, data) => ada.request.post(path, { data, headers: { origin: server.url } });
    const { id } = await (await post("/api/v1/spaces/family/documents", { name: "Minutes" })).json();
    const writer = await open(ada, id);
    await writer.keyboard.type("For the accountant.");
    await expect(tools(writer).getByText("Saved", { exact: true })).toBeAttached();
    const link = async (label, role) => {
      const made = await post(`/api/v1/documents/${id}/links`, { label, role, allows: { copy: true } });
      expect(made.ok(), label).toBe(true);
      return (await made.json()).url;
    };

    // A reader's link shows the document on a plain page: no editor opens for it.
    const reader = await stranger(browser, server);
    await reader.goto(await link("Accountant", "reader"));
    expect(new URL(reader.url()).pathname).toBe(`/d/${id}/read`);
    await expect(reader.getByText("For the accountant.")).toBeVisible();
    await expect(reader.locator("canvas")).toHaveCount(0);

    // A commenter's link opens the editor, which says what they may do.
    const guest = await stranger(browser, server);
    await guest.goto(await link("Solicitor", "commenter"));
    await guest.waitForFunction(() => window.letters?.frames > 0 && window.letters.live, undefined, { timeout: 60_000 });
    expect(new URL(guest.url()).pathname).toBe(`/d/${id}`);
    await expect.poll(() => shown(guest)).toContain("For the accountant.");
    await expect(tools(guest).getByText("Commenting", { exact: true })).toBeAttached();
    // The writer sees who is there, by the link's label, and what is written reaches them.
    await expect(writer.getByLabel("Solicitor")).toBeAttached();
    await writer.bringToFront();
    await writer.keyboard.type(" And more.");
    await expect.poll(() => shown(guest)).toContain("And more.");

    // The link lets them copy, and neither print nor take a file, and they are shown no history.
    await guest.bringToFront();
    await expect(guest.getByRole("button", { name: "History", exact: true })).toHaveCount(0);
    const more = await guest.getByRole("button", { name: "More", exact: true }).boundingBox();
    await guest.mouse.click(more.x + more.width / 2, more.y + more.height / 2);
    await expect(guest.getByRole("menuitem", { name: "Settings" })).toBeAttached();
    await expect(guest.getByRole("menuitem", { name: "Export" })).toHaveCount(0);
    await guest.emulateMedia({ media: "print" });
    await expect(guest.locator("canvas")).toBeHidden();
    expect(await guest.evaluate(() => window.blocked)).toEqual([]);
  } finally {
    server.stop();
  }
});

test("a word and a source of pictures a person adds in one browser are theirs in the next", async ({ browser }) => {
  const server = await served();
  try {
    const ada = await server.browser(browser, "ada");
    const made = await ada.request.post("/api/v1/spaces/family/documents", { data: { name: "Minutes" }, headers: { origin: server.url } });
    const { id } = await made.json();
    const settings = async (page) => {
      await page.keyboard.press("ControlOrMeta+,");
      return page.getByRole("dialog", { name: "Settings" });
    };
    const first = await open(ada, id);
    await learn(first, "Zorvex");
    const field = await (await settings(first)).getByRole("textbox", { name: "Trust a source" }).boundingBox();
    await first.mouse.click(field.x + field.width / 2, field.y + field.height / 2);
    await first.keyboard.type("img.example.org");
    await first.keyboard.press("Enter");
    await expect(first.getByRole("listitem", { name: "https://img.example.org/" })).toBeAttached();

    // Another browser of theirs has kept nothing: what it lists is the account's.
    const next = await open(await server.browser(browser, "ada"), id);
    const dialog = await settings(next);
    for (const name of ["Zorvex", "https://img.example.org/"]) {
      await expect(dialog.getByRole("listitem", { name })).toBeAttached();
    }
    expect(await next.evaluate(() => window.blocked)).toEqual([]);
  } finally {
    server.stop();
  }
});

test("who wrote since a person's last visit is told from what their account keeps, in whatever browser", async ({ browser }) => {
  const server = await served();
  try {
    const ada = await server.browser(browser, "ada");
    const made = await ada.request.post("/api/v1/spaces/family/documents", { data: { name: "Minutes" }, headers: { origin: server.url } });
    const { id } = await made.json();
    // A page that goes says where its person read up to, though nothing was written on it.
    const first = await open(ada, id);
    await first.goto("about:blank");
    await expect.poll(async () => (await ada.request.get(`/d/${id}`)).text()).toContain('"last_seen"');

    server.device(`join ${id}`);
    await server.texts();
    server.device("write From the desk. ");
    await expect.poll(async () => (await server.texts()).join("\n")).toContain("From the desk.");

    const next = await open(await server.browser(browser, "ada"), id);
    await expect(next.getByText("Edited by Ben", { exact: true })).toBeAttached();
  } finally {
    server.stop();
  }
});

test("text in a language the server keeps a dictionary for is checked against it, in whatever encoding the files are", async ({ browser }) => {
  const server = await served();
  try {
    // A dictionary as a system keeps it: named for its language and country, in an older encoding.
    const kept = join(server.folder, "state", "dictionaries");
    mkdirSync(kept, { recursive: true });
    writeFileSync(join(kept, "de_DE.aff"), Buffer.from("SET ISO8859-1\nTRY esn\n", "latin1"));
    writeFileSync(join(kept, "de_DE.dic"), Buffer.from("2\nStraße\nÜbung\n", "latin1"));
    const ada = await server.browser(browser, "ada");
    const made = await ada.request.post("/api/v1/spaces/family/documents", { data: { name: "Brief" }, headers: { origin: server.url } });
    const { id } = await made.json();
    const page = await open(ada, id);

    // The dialog offers what the server has. Nothing was fetched for a language nothing is written in.
    const fetched = [];
    page.on("response", (response) => /\/dictionaries\/.+\/(aff|dic)$/.test(response.url()) && fetched.push(response));
    await page.keyboard.press("ControlOrMeta+Shift+p");
    await page.keyboard.type("Language");
    await rested(page);
    await page.keyboard.press("Enter");
    const press = async (role, name) => {
      const box = await page.getByRole(role, { name, exact: true }).boundingBox();
      await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    };
    await press("button", "Language of the document");
    await press("menuitemcheckbox", "German (Germany)");
    await page.keyboard.press("Escape");
    // The language is written in now, so its dictionary is fetched.
    await expect.poll(() => fetched.length).toBe(2);
    await Promise.all(fetched.map((response) => response.finished()));
    await rested(page);

    // A wrong word in the dictionary's own letters is marked, and its correction is the dictionary's.
    await page.keyboard.type("Strase ");
    await rested(page);
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    await page.locator("textarea").dispatchEvent("contextmenu");
    await expect(page.getByRole("menuitem", { name: "Straße" })).toBeAttached();
    expect(fetched.map((response) => new URL(response.url()).pathname).sort()).toEqual(["/api/v1/dictionaries/de-DE/aff", "/api/v1/dictionaries/de-DE/dic"]);
    expect(fetched.map((response) => response.status())).toEqual([200, 200]);
    expect(await page.evaluate(() => window.blocked)).toEqual([]);
  } finally {
    server.stop();
  }
});

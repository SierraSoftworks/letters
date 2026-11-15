// The served page's line to its server: what is written goes to the document's room, and what the others
// write comes from it. The room is `letters_sync::Room` itself, behind the page's socket.

import { expect, test } from "@playwright/test";

import { blocks, carry, png, rested, texts } from "./page.js";
import { opened, room, serve } from "./server.js";

const rooms = [];
/** A room that is gone with the test. */
const hosted = (document) => {
  const host = room(opened(), document);
  rooms.push(host);
  return host;
};
test.afterEach(() => rooms.splice(0).forEach((host) => host.close()));

/** A document of one line under its heading. */
const minutes = (text) => ({
  format: 1,
  language: "en",
  blocks: [
    { id: "h", type: "heading", level: 1, content: [{ text: "Minutes" }] },
    { id: "p", type: "paragraph", content: [{ text }] },
  ],
});

/** How the bar says the document stands to its server. */
const mark = (page, said) => page.getByRole("toolbar", { name: "Document" }).getByText(said, { exact: true });
const held = async (host) => (await host.texts()).join("\n");
const shown = async (page) => (await texts(page)).join("\n");

test("what is written on the server's page is kept by the server, and the bar says so", async ({ page }) => {
  const host = hosted();
  await serve(page, opened(), undefined, undefined, host);
  await page.keyboard.type("Written here.");
  await expect.poll(() => held(host)).toContain("Written here.");
  await expect(mark(page, "Saved")).toBeAttached();
  expect(await page.evaluate(() => window.blocked)).toEqual([]);
});

test("two people with the document open read what the other writes, and see who is there", async ({ page, context }) => {
  const [host, other] = [hosted(), await context.newPage()];
  await serve(page, opened(), undefined, undefined, host);
  await serve(other, opened({ author: "Grace Hopper" }), undefined, undefined, host);
  await expect(page.getByLabel("Grace Hopper")).toBeAttached();
  await expect(other.getByLabel("Ada Lovelace")).toBeAttached();

  await page.keyboard.type("From Ada.");
  await expect.poll(() => shown(other)).toContain("From Ada.");
  await other.bringToFront();
  await other.keyboard.press("Enter");
  await other.keyboard.type("From Grace.");
  await expect.poll(() => shown(page)).toContain("From Grace.");
  // Both hold the one document, which is the server's.
  await expect.poll(() => held(host)).toContain("From Grace.");
  expect(await blocks(page)).toEqual(await blocks(other));

  // Someone who leaves is no longer shown.
  await other.goto("about:blank");
  await expect(page.getByLabel("Grace Hopper")).not.toBeAttached();
});

test("someone who may only read the document reads what is written and writes nothing", async ({ page, context }) => {
  const [host, reader] = [hosted(), await context.newPage()];
  await serve(page, opened(), undefined, undefined, host);
  await serve(reader, opened({ role: "reader", author: "Grace Hopper" }), undefined, undefined, host);
  await reader.keyboard.type("Not mine to write.");
  await page.bringToFront();
  await page.keyboard.type("From Ada.");
  await expect.poll(() => shown(reader)).toContain("From Ada.");
  expect(await shown(reader)).not.toContain("Not mine to write.");
  expect(await held(host)).not.toContain("Not mine to write.");
});

test("a page written before the document changed shows the document as the server holds it", async ({ page }) => {
  const host = hosted(minutes("As the server holds it."));
  await serve(page, opened(), undefined, minutes("As the page was written."), host);
  expect(await shown(page)).toContain("As the server holds it.");
  expect(await shown(page)).not.toContain("As the page was written.");
  await page.keyboard.type(" And on.");
  await expect.poll(() => held(host)).toContain("As the server holds it. And on.");
});

test("what is written while the server cannot be reached is handed over when it can", async ({ page }) => {
  const host = hosted();
  await serve(page, opened(), undefined, undefined, host);
  host.cut();
  await expect(mark(page, /^(Offline|Connecting)$/)).toBeAttached();
  await page.keyboard.type("Written while cut off.");
  // What only the page holds can be kept as a file meanwhile.
  await expect(page.getByRole("button", { name: "Save", exact: true })).toBeAttached();
  expect(await held(host)).not.toContain("Written while cut off.");

  host.mend();
  await expect.poll(() => held(host), { timeout: 15_000 }).toContain("Written while cut off.");
  await expect(mark(page, "Saved")).toBeAttached();
});

test("a page that has not reached its server yet is read and not written, and says so after a moment", async ({ page }) => {
  const host = hosted();
  host.cut();
  await serve(page, opened(), undefined, undefined, host);
  await page.keyboard.type("Too early.");
  await rested(page);
  expect(await shown(page)).not.toContain("Too early.");
  await expect(mark(page, /^(Offline|Connecting)$/)).toBeAttached();

  host.mend();
  await page.waitForFunction(() => window.letters.live, undefined, { timeout: 15_000 });
  await page.keyboard.type("In time.");
  await expect.poll(() => held(host)).toContain("In time.");
  await expect(mark(page, "Saved")).toBeAttached();
});

test("a picture is the server's before the edit that names it is", async ({ page }) => {
  const host = hosted();
  let keep;
  const kept = new Promise((done) => (keep = done));
  const asked = await serve(page, opened(), (method) => (method === "PUT" ? kept : undefined), undefined, host);
  await carry(page, "textarea", "paste", [["shot.png", "image/png", await png(page)]]);
  await expect.poll(() => asked.filter(({ method }) => method === "PUT")).toHaveLength(1);
  await expect.poll(async () => (await blocks(page)).some((block) => block.type === "figure")).toBe(true);
  // Long enough for an edit to reach the room, as the first test's does.
  await page.waitForTimeout(500);
  const figures = async () => (await host.document()).blocks.filter((block) => block.type === "figure").length;
  expect(await figures()).toBe(0);

  keep({ status: 204 });
  await expect.poll(figures).toBe(1);
});

test("a comment written by one is read by the other", async ({ page, context }) => {
  const [host, other] = [hosted(minutes("A line to remark on.")), await context.newPage()];
  await serve(page, opened(), undefined, minutes("A line to remark on."), host);
  await serve(other, opened({ author: "Grace Hopper" }), undefined, minutes("A line to remark on."), host);
  await page.bringToFront();
  await page.keyboard.press("Shift+Home");
  await page.keyboard.press("ControlOrMeta+Shift+p");
  await page.keyboard.type("Comment");
  await rested(page);
  await page.keyboard.press("Enter");
  await page.keyboard.type("Is this the line?");
  await page.getByRole("button", { name: "Send" }).dispatchEvent("click");
  const marker = other.getByRole("button", { name: "1 comment" });
  await expect(marker).toBeAttached();
  await marker.dispatchEvent("click");
  await expect(other.getByLabel("Is this the line?")).toBeAttached();
});

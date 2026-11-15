// The page a server sends for a document it keeps: the editor opens it as the server says.

import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { createHash } from "node:crypto";

import { INK, blocks, carry, entry, file, inked, png, rested, texts } from "./page.js";
import { album, opened, picture, serve } from "./server.js";

const DOCUMENT = `/api/v1/documents/${opened().id}`;
const TRUST = `${DOCUMENT}/trust`;
const bar = (page) => page.getByText(/^Safe mode:/);

test("the server's page opens its document as the server says, under a policy that lets no inline script run", async ({ page }) => {
  const said = opened({ style: { format: 1, page: { margin_top: 99 } } });
  await serve(page, said);
  await expect(page).toHaveTitle("Minutes");
  expect(await texts(page)).toContain("Letters in the browser");
  await expect(bar(page)).not.toBeAttached();

  // What is kept from here is the server's document: the same one, in its own style.
  await page.keyboard.type("Written here.");
  const kept = page.waitForEvent("download");
  await page.keyboard.press("ControlOrMeta+s");
  const file = await kept;
  expect(file.suggestedFilename()).toBe("Minutes.let");
  const zip = await readFile(await file.path());
  expect(JSON.parse(entry(zip, "manifest.json"))).toMatchObject({ id: said.id, created: said.created });
  expect(JSON.parse(entry(zip, "style.json")).page.margin_top).toBe(99);

  expect(await page.evaluate(() => window.blocked)).toEqual([]);
});

const answers = [
  { name: "agrees", answer: { status: 204 }, held: false, told: null },
  {
    name: "refuses",
    answer: { status: 403, body: { code: "forbidden", message: "Only its owner may trust this document." } },
    held: true,
    told: "Only its owner may trust this document.",
  },
  { name: "cannot be read", answer: { status: 502, body: "<html>Bad gateway</html>", type: "text/html" }, held: true, told: /502/ },
];
for (const { name, answer, held, told } of answers) {
  test(`a document the server holds in safe mode is trusted through the server, which ${name}`, async ({ page }) => {
    const said = [];
    page.on("dialog", (dialog) => (said.push(dialog.message()), dialog.accept()));
    const asked = await serve(page, opened({ trust: "safe" }), (method, path) => (path === TRUST ? answer : undefined));
    await expect(bar(page)).toBeAttached();

    // Where the canvas draws the button.
    const box = await page.getByRole("button", { name: "Trust this document" }).boundingBox();
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await expect.poll(() => asked.map(({ method, path }) => `${method} ${path}`)).toEqual([`POST ${TRUST}`]);
    if (told) {
      await expect.poll(() => said).toHaveLength(1);
      expect(said[0]).toMatch(told);
    }
    await rested(page);
    await expect(bar(page)).toBeAttached({ attached: held });
    expect(said).toHaveLength(told ? 1 : 0);
  });
}

test("the pictures of the server's document come from the server, those the page never showed too", async ({ page }) => {
  // The second lies far below what the window shows.
  const [shown, below] = [picture(INK), picture([0, 128, 255], 64, 4000)];
  const kept = new Map([shown, below].map(({ name, bytes }) => [`${DOCUMENT}/${name}`, bytes]));
  const answer = (method, path) => (method === "GET" && kept.has(path) ? { body: kept.get(path), type: "image/png" } : undefined);
  const asked = await serve(page, opened(), answer, album([shown.name, below.name]));
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);

  // A copy of the document leaves none of them out.
  const download = page.waitForEvent("download");
  await page.keyboard.press("ControlOrMeta+s");
  const zip = await readFile(await (await download).path());
  for (const { name, bytes } of [shown, below]) {
    expect(file(zip, name)?.equals(bytes), name).toBe(true);
  }
  // Each was asked for once, whoever wanted it.
  expect(asked.map(({ method, path }) => `${method} ${path}`).sort()).toEqual([...kept.keys()].map((path) => `GET ${path}`).sort());
});

test("a copy is not made of a document whose picture the server does not give", async ({ page }) => {
  const said = [];
  page.on("dialog", (dialog) => (said.push(dialog.message()), dialog.accept()));
  let downloads = 0;
  page.on("download", () => downloads++);
  await serve(page, opened(), undefined, album([picture(INK).name]));
  await page.keyboard.press("ControlOrMeta+s");
  await expect.poll(() => said).toHaveLength(1);
  expect(said[0]).toMatch(/could not be saved.*picture/);
  expect(downloads).toBe(0);
});

const keeps = [
  { name: "keeps", answer: { status: 204 }, told: 0 },
  { name: "refuses", answer: { status: 413, body: { code: "too_large", message: "A picture may be 32 MB at most." } }, told: 1 },
];
for (const { name, answer, told } of keeps) {
  test(`a picture put into the server's document is handed to the server, which ${name} it`, async ({ page }) => {
    const said = [];
    page.on("dialog", (dialog) => (said.push(dialog.message()), dialog.accept()));
    const asked = await serve(page, opened(), (method) => (method === "PUT" ? answer : undefined));
    const bytes = Buffer.from(await png(page));
    await carry(page, "textarea", "paste", [["shot.png", "image/png", [...bytes]]]);
    await expect.poll(async () => (await blocks(page)).some((block) => block.type === "figure")).toBe(true);

    // Under the name the document has for it, which is the hash of its bytes.
    const hash = createHash("sha256").update(bytes).digest("hex");
    await expect.poll(() => asked.map(({ method, path }) => `${method} ${path}`)).toEqual([`PUT ${DOCUMENT}/assets/${hash}.png`]);
    expect(asked[0].body.equals(bytes)).toBe(true);
    expect(asked[0].type).toBe("image/png");
    // The picture is on the page either way, and the person is told when it is nowhere else.
    await expect.poll(() => inked(page)).toBeGreaterThan(1000);
    await expect.poll(() => said).toEqual(told ? ["A picture may be 32 MB at most."] : []);
  });
}

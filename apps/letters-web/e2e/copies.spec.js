// Save a copy: the document again, or in another format, as a file the browser keeps for the person.

import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { blocks, ready, rested } from "./page.js";

const card = (page) => page.getByRole("dialog", { name: "Save a copy" });

/** Presses the control of the card called `name`, where the mirror says it is. */
const click = async (page, name) => {
  const box = await card(page).getByRole("button", { name, exact: true }).boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
};

/** Opens the card, and sets it to a copy called `name` in `format`. */
const ask = async (page, name, format) => {
  await page.keyboard.press("ControlOrMeta+Shift+s");
  await expect(card(page)).toBeAttached();
  await page.keyboard.type(name);
  await click(page, format);
  await rested(page);
};

/** The file the browser is handed when the card is told to save: its name, and its bytes as text. */
const saved = async (page) => {
  const kept = page.waitForEvent("download");
  await page.keyboard.press("Enter");
  const file = await kept;
  return [file.suggestedFilename(), (await readFile(await file.path())).toString("latin1")];
};

/** Pastes a small picture, which becomes a figure. */
const picture = async (page) => {
  await page.evaluate(async () => {
    const canvas = Object.assign(document.createElement("canvas"), { width: 32, height: 24 });
    canvas.getContext("2d").fillRect(0, 0, 32, 24);
    const blob = await new Promise((done) => canvas.toBlob(done, "image/png"));
    const data = new DataTransfer();
    data.items.add(new File([blob], "shot.png", { type: "image/png" }));
    const paste = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true });
    document.querySelector("textarea").dispatchEvent(paste);
  });
  await expect.poll(async () => (await blocks(page)).some((block) => block.type === "figure")).toBe(true);
};

test.beforeEach(({ page }) => ready(page));

for (const [format, ending, starts, ...holds] of [
  // A document of its own is an archive that says what it is first, and brings its history.
  ["Letters", "let", "PK", "mimetypeapplication/vnd.letters.document+zip", "history/oplog"],
  ["Word", "docx", "PK", "word/document.xml"],
  ["PDF", "pdf", "%PDF-", "/Title"],
  ["Web page", "html", "<!doctype html>", "Kept from a browser"],
  ["Markdown", "md", "", "Kept from a browser"],
]) {
  test(`a copy as ${format} is a file the browser keeps`, async ({ page }) => {
    await page.keyboard.type("Kept from a browser");
    await ask(page, "Kept: a copy", format);
    const [name, file] = await saved(page);
    // The name is what was typed, less what no file can be called.
    expect(name).toBe(`Kept a copy.${ending}`);
    expect(file.startsWith(starts)).toBe(true);
    for (const held of holds) expect(file).toContain(held);
    await expect(card(page)).not.toBeAttached();
  });
}

test("a Markdown copy brings its pictures with it", async ({ page }) => {
  await picture(page);
  await ask(page, "Shots", "Markdown");
  const [name, file] = await saved(page);
  expect(name).toBe("Shots.zip");
  expect(file.startsWith("PK")).toBe(true);
  expect(file).toContain("Shots.md");
  expect(file).toMatch(/assets\/[^/]+\.png/);
});

test("what writes a copy is fetched only when one is asked for", async ({ page }) => {
  const fetched = [];
  page.on("request", (request) => fetched.push(new URL(request.url()).pathname));
  await ask(page, "Late", "Web page");
  expect(fetched.filter((path) => path.includes("export"))).toEqual([]);
  await saved(page);
  expect(fetched).toContain("/export_bg.wasm");
});

test("a copy that cannot be written leaves its card to ask again", async ({ page }) => {
  let downloads = 0;
  page.on("download", () => downloads++);
  await page.route("**/export.js*", (route) => route.abort());
  await ask(page, "Later", "Web page");
  const failed = page.waitForEvent("requestfailed");
  await page.keyboard.press("Enter");
  await failed;
  await rested(page);
  expect(downloads).toBe(0);

  // The card was put away while the copy was made, and is back as it was to be told again.
  await page.unroute("**/export.js*");
  const [name] = await saved(page);
  expect(name).toBe("Later.html");
  await expect(card(page)).not.toBeAttached();
});

// A document's file: saved from the page, chosen after "Open", or let go on the page.

import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { blocks, entry, history, ready, rested, texts } from "./page.js";

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

/**
 * Writes a document in the page and saves a copy of it: the copy's file, and the history of the document
 * it was made from. The page is then as it was first loaded.
 */
const written = async (page, text, name = "Kept") => {
  await page.keyboard.type(text);
  await picture(page);
  await page.keyboard.press("ControlOrMeta+Shift+s");
  await expect(page.getByRole("dialog", { name: "Save a copy" })).toBeAttached();
  await page.keyboard.type(name);
  const kept = page.waitForEvent("download");
  await page.keyboard.press("Enter");
  const file = { name: `${name}.let`, mimeType: "", buffer: await readFile(await (await kept).path()) };
  const history = await page.evaluate(() => [...window.letters.history()]);
  await ready(page);
  return { file, history };
};

/** Lets files go on the page. */
const drop = (page, files) =>
  page.evaluate((files) => {
    const data = new DataTransfer();
    for (const { name, mimeType, bytes } of files) data.items.add(new File([new Uint8Array(bytes)], name, { type: mimeType }));
    const at = { clientX: 550, clientY: 300, bubbles: true, cancelable: true };
    document.getElementById("letters").dispatchEvent(new DragEvent("drop", { dataTransfer: data, ...at }));
  }, files.map((file) => ({ ...file, bytes: [...file.buffer] })));

/** The file the browser is handed when `key` is pressed: its name, and its bytes. */
const saved = async (page, key = "ControlOrMeta+s") => {
  const kept = page.waitForEvent("download");
  await page.keyboard.press(key);
  const file = await kept;
  return { name: file.suggestedFilename(), mimeType: "", buffer: await readFile(await file.path()) };
};

/** What makes a file's document that one and no other. */
const identity = (file) => {
  const { id, created } = JSON.parse(entry(file.buffer, "manifest.json"));
  return { id, created };
};

test.beforeEach(({ page }) => ready(page));

test("Ctrl or Cmd with S keeps the document as its own file, which stays the same document however often", async ({ page }) => {
  await page.evaluate(() => {
    window.saves = [];
    const heard = (event) => event.key === "s" && window.saves.push(event.defaultPrevented);
    window.addEventListener("keydown", heard);
  });
  await page.keyboard.type("Saved by hand");
  const first = await saved(page);
  // The browser's own answer to the key would save the page.
  expect(await page.evaluate(() => window.saves)).toEqual([true]);
  expect(first.name).toBe(`${await page.title()}.let`);
  expect(entry(first.buffer, "document.json")).toContain("Saved by hand");

  await page.keyboard.type(", and again");
  const second = await saved(page);
  expect(entry(second.buffer, "document.json")).toContain("Saved by hand, and again");
  expect(identity(second)).toEqual(identity(first));
});

test("the bar offers to save what was written on the page, until it is kept", async ({ page }) => {
  const save = page.getByRole("button", { name: "Save", exact: true });
  await expect(save).toHaveCount(0);

  await page.keyboard.type("Kept from the bar");
  await save.waitFor({ state: "attached" });
  const kept = page.waitForEvent("download");
  await save.dispatchEvent("click");
  const file = await readFile(await (await kept).path());
  expect(entry(file, "document.json")).toContain("Kept from the bar");
  await expect(save).toHaveCount(0);
});

test("a file that was opened is saved as the same document, and copied as another", async ({ page }) => {
  await page.keyboard.type("From one page");
  const file = await saved(page);
  await ready(page);
  await drop(page, [file]);
  await expect.poll(() => texts(page)).toContain("From one page");

  await page.keyboard.type("Written in another. ");
  const again = await saved(page);
  expect(entry(again.buffer, "document.json")).toContain("Written in another.");
  expect(identity(again)).toEqual(identity(file));

  await page.keyboard.press("ControlOrMeta+Shift+s");
  await expect(page.getByRole("dialog", { name: "Save a copy" })).toBeAttached();
  const copy = await saved(page, "Enter");
  expect(identity(copy).id).not.toBe(identity(file).id);
});

test("what a file held when it was opened is a version of its own once it is written on", async ({ page }) => {
  await page.keyboard.type("From one sitting");
  const file = await saved(page);
  await ready(page);
  await drop(page, [file]);
  await expect.poll(() => texts(page)).toContain("From one sitting");

  // A version between the first and the current one is listed by the time it was written at.
  const earlier = page.getByRole("option", { name: /^\d\d:\d\d$/ });
  await history(page);
  await expect(earlier).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.keyboard.type("And from the next. ");
  await history(page);
  await expect(earlier).toHaveCount(1);
});

test("Open asks for a document's file, and shows it with its name, its pictures and its history", async ({ page }) => {
  const { file, history } = await written(page, "Written before it was kept");
  expect(await texts(page)).not.toContain("Written before it was kept");

  await page.keyboard.press("ControlOrMeta+Shift+p");
  await page.keyboard.type("Open");
  // The browser is told to hand the chooser over before the key that asks for it.
  const asked = page.waitForEvent("filechooser");
  await rested(page);
  await page.keyboard.press("Enter");
  await (await asked).setFiles(file);

  await expect.poll(() => texts(page)).toContain("Written before it was kept");
  expect((await blocks(page)).some((block) => block.type === "figure")).toBe(true);
  await expect(page).toHaveTitle("Kept");
  // It is the document the copy was made of: what that one knew is nothing new to this one.
  const before = await texts(page);
  await page.evaluate((history) => window.letters.merge(new Uint8Array(history)), history);
  expect(await texts(page)).toEqual(before);

  // The picture came with the file, and goes with the next copy.
  await page.keyboard.press("ControlOrMeta+Shift+s");
  await expect(page.getByRole("dialog", { name: "Save a copy" })).toBeAttached();
  const kept = page.waitForEvent("download");
  await page.keyboard.press("Enter");
  expect((await readFile(await (await kept).path())).toString("latin1")).toMatch(/assets\/[^/]+\.png/);
});

test("a document's file let go on the page is opened, once the person agrees to lose what they wrote there", async ({ page }) => {
  const { file } = await written(page, "Brought in by hand");
  await page.keyboard.type("Not kept anywhere");

  /** Lets the file go on the page, and answers what the page asks before it opens it. */
  const answer = async (agree) => {
    const asked = page.waitForEvent("dialog");
    // The page waits for the answer, so the drop is not over until it is given.
    const dropped = drop(page, [file]);
    const ask = await asked;
    expect(ask.type()).toBe("confirm");
    expect(ask.message()).toContain("Kept");
    await (agree ? ask.accept() : ask.dismiss());
    await dropped;
  };
  await answer(false);
  await rested(page);
  expect(await texts(page)).toContain("Not kept anywhere");

  await answer(true);
  await expect.poll(() => texts(page)).toContain("Brought in by hand");
  expect(await texts(page)).not.toContain("Not kept anywhere");

  // Nothing was written in what is now on the page, so nothing is asked before it gives way again.
  let asked = 0;
  page.on("dialog", () => asked++);
  await page.keyboard.press("ControlOrMeta+a");
  await drop(page, [file]);
  await rested(page);
  expect(asked).toBe(0);
});

test("a file that is no document is said to be none, and the page stays as it is", async ({ page }) => {
  const shown = await texts(page);
  const said = page.waitForEvent("dialog");
  await drop(page, [{ name: "Torn.let", mimeType: "", buffer: Buffer.from("not a document") }]);
  const alert = await said;
  expect(alert.message()).toContain("Torn");
  expect(alert.message()).toContain("could not be opened");
  await alert.dismiss();
  expect(await texts(page)).toEqual(shown);
});

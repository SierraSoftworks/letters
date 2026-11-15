// Pictures on their way into the document, and on the page once they are in it.

import { expect, test } from "@playwright/test";

import { blocks, ready, rested } from "./page.js";

/** The colour every test picture is filled with, which nothing else in the window is. */
const INK = [255, 0, 255];

/** A PNG of one colour, as the numbers of its bytes. */
const png = (page, width = 64, height = 48) =>
  page.evaluate(
    async ([width, height, ink]) => {
      const canvas = Object.assign(document.createElement("canvas"), { width, height });
      const context = canvas.getContext("2d");
      context.fillStyle = `rgb(${ink})`;
      context.fillRect(0, 0, width, height);
      const blob = await new Promise((done) => canvas.toBlob(done, "image/png"));
      return [...new Uint8Array(await blob.arrayBuffer())];
    },
    [width, height, INK],
  );

/** Raises `kind` on what `selector` finds, carrying files: each a name, a type and its bytes. */
const carry = (page, selector, kind, files, at = {}) =>
  page.evaluate(
    ([selector, kind, files, at]) => {
      const data = new DataTransfer();
      for (const [name, type, bytes] of files) data.items.add(new File([new Uint8Array(bytes)], name, { type }));
      const carried = kind === "paste" ? { clipboardData: data } : { dataTransfer: data, ...at };
      const Kind = kind === "paste" ? ClipboardEvent : DragEvent;
      return document.querySelector(selector).dispatchEvent(new Kind(kind, { ...carried, bubbles: true, cancelable: true }));
    },
    [selector, kind, files, at],
  );

const figures = async (page) => (await blocks(page)).filter((block) => block.type === "figure").length;

/** How many pixels of the canvas are in the pictures' colour. */
const inked = (page) =>
  page.evaluate((ink) => {
    const canvas = document.getElementById("letters");
    const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
    let count = 0;
    for (let at = 0; at < pixels.length; at += 4) {
      if (pixels[at] === ink[0] && pixels[at + 1] === ink[1] && pixels[at + 2] === ink[2]) count++;
    }
    return count;
  }, INK);

test.beforeEach(({ page }) => ready(page));

test("a pasted picture becomes a figure, and is drawn on the page", async ({ page }) => {
  expect(await inked(page)).toBe(0);
  await carry(page, "textarea", "paste", [["shot.png", "image/png", await png(page)]]);
  await expect.poll(() => figures(page)).toBe(1);
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);
});

test("pictures dropped on the page land there, and files that are no pictures do not", async ({ page }) => {
  const at = { clientX: 550, clientY: 300 };
  const picture = ["photo.png", "image/png", await png(page)];
  // Held over the page, they may be let go there.
  expect(await carry(page, "#letters", "dragover", [picture], at)).toBe(false);
  await carry(page, "#letters", "drop", [picture, ["notes.txt", "text/plain", [104, 105]]], at);
  await expect.poll(() => figures(page)).toBe(1);
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);

  await carry(page, "#letters", "drop", [["notes.png", "image/png", [104, 105]]], at);
  await rested(page);
  expect(await figures(page)).toBe(1);
});

test("Image in the insert menu asks for files, and puts their pictures in at the caret", async ({ page }) => {
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("/image");
  // The browser is told to hand the chooser over before the key that asks for it.
  const asked = page.waitForEvent("filechooser");
  await rested(page);
  await page.keyboard.press("Enter");
  const chooser = await asked;
  expect(chooser.isMultiple()).toBe(true);
  await chooser.setFiles({ name: "chosen.png", mimeType: "image/png", buffer: Buffer.from(await png(page)) });
  await expect.poll(() => figures(page)).toBe(1);
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);
});

test("a picture stays on the page when its document is opened again", async ({ page }) => {
  await carry(page, "textarea", "paste", [["shot.png", "image/png", await png(page)]]);
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);
  await page.evaluate(() => window.letters.open(window.letters.json()));
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);
});

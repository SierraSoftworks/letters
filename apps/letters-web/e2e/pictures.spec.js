// Pictures on their way into the document, and on the page once they are in it.

import { expect, test } from "@playwright/test";

import { blocks, carry, inked, png, ready, rested } from "./page.js";

const figures = async (page) => (await blocks(page)).filter((block) => block.type === "figure").length;

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

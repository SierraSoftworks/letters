// Emoji and the scripts the fonts lack: a face that has them is fetched when the page first shows one.

import { existsSync } from "node:fs";

import { expect, test } from "@playwright/test";

import { ready, rested } from "./page.js";

// The faces are fetched as the page is built, and a build without a network has none.
const built = existsSync(new URL("../dist/fallback/NotoColorEmoji.ttf", import.meta.url));
test.skip(!built, "this build has no faces to back up the fonts");

/** The faces the page asks for from here on, by their files, in the order it asks. */
const faces = (page) => {
  const asked = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.startsWith("/fallback/")) asked.push(path.slice("/fallback/".length));
  });
  return asked;
};

/** How many of the canvas's pixels are the yellow of an emoji's face, which nothing else on the page is. */
const yellow = (page) =>
  page.evaluate(() => {
    const canvas = document.querySelector("canvas");
    const { data } = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height);
    let count = 0;
    for (let at = 0; at < data.length; at += 4) if (data[at] > 220 && data[at + 1] > 170 && data[at + 2] < 110) count++;
    return count;
  });

test("an emoji is drawn in colour, in a face fetched when the page first shows one", async ({ page }) => {
  const asked = faces(page);
  await ready(page);
  await page.keyboard.type("Plain words");
  await rested(page);
  expect(asked).toEqual([]);
  expect(await yellow(page)).toBe(0);

  await page.keyboard.insertText(" 🙂");
  await expect.poll(() => yellow(page)).toBeGreaterThan(0);
  await page.keyboard.insertText(" 🎉");
  await rested(page);
  expect(asked).toEqual(["NotoColorEmoji.ttf"]);
});

test("Han is fetched as its neighbours are written, and what that face lacks from the next", async ({ page }) => {
  const asked = faces(page);
  await ready(page);
  await page.keyboard.insertText("日本語のテキスト");
  await expect.poll(() => asked).toEqual(["NotoSansJP.otf"]);
  // A character only Chinese writes.
  await page.keyboard.insertText("汉");
  await expect.poll(() => asked).toEqual(["NotoSansJP.otf", "NotoSansSC.otf"]);
});

for (const [script, text, face] of [
  ["Arabic", "العربية", "NotoSansArabic.ttf"],
  ["Hebrew", "עברית", "NotoSansHebrew.ttf"],
  ["Devanagari", "हिन्दी", "NotoSansDevanagari.ttf"],
  ["Thai", "ภาษาไทย", "NotoSansThai.ttf"],
]) {
  test(`${script} is drawn in a face of its own`, async ({ page }) => {
    const asked = faces(page);
    await ready(page);
    await page.keyboard.insertText(text);
    await expect.poll(() => asked).toEqual([face]);
  });
}

test("a face that cannot be fetched is done without, and not asked for again", async ({ page }) => {
  await page.route("**/fallback/NotoColorEmoji.ttf", (route) => route.abort());
  const asked = faces(page);
  await ready(page);
  await page.keyboard.insertText("🙂");
  await expect.poll(() => asked).toEqual(["NotoColorEmoji.ttf"]);
  await page.keyboard.insertText(" 🎉 and on");
  await rested(page);
  expect(asked).toEqual(["NotoColorEmoji.ttf"]);
  expect(await page.evaluate(() => JSON.parse(window.letters.json()).blocks.at(-1).content[0].text)).toContain("and on");
});

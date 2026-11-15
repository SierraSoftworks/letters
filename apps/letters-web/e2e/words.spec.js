// Thai, Lao, Khmer and Burmese are written without spaces: a line of one ends between words by the script's
// word list, which is fetched when the page first shows the script.

import { expect, test } from "@playwright/test";

import { ready, rested, texts } from "./page.js";

/** The word lists the page asks for from here on, by their scripts, in the order it asks. */
const lists = (page) => {
  const asked = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.startsWith("/words/")) asked.push(path.slice("/words/".length).replace(/-.*/, ""));
  });
  return asked;
};

/** A Thai sentence, word by word. */
const WORDS = ["ภาษา", "ไทย", "เขียน", "ติด", "กัน", "โดย", "ไม่", "เว้น", "วรรค", "ระหว่าง", "คำ"];
const SENTENCE = WORDS.join("");

for (const [script, text] of [
  ["thai", "ภาษาไทย"],
  ["lao", "ພາສາລາວ"],
  ["khmer", "ភាសាខ្មែរ"],
  ["burmese", "မြန်မာဘာသာ"],
]) {
  test(`the ${script} word list is fetched when the page first shows the script, and once`, async ({ page }) => {
    const asked = lists(page);
    await ready(page);
    await page.keyboard.type("Plain words");
    await rested(page);
    expect(asked).toEqual([]);

    await page.keyboard.insertText(text);
    await expect.poll(() => asked).toEqual([script]);
    await page.keyboard.insertText(text);
    await rested(page);
    expect(asked).toEqual([script]);
  });
}

test("a line of Thai ends between words once its list is here", async ({ page }) => {
  const arrived = page.waitForResponse("**/words/thai-*");
  await ready(page);
  await page.keyboard.insertText(SENTENCE.repeat(12));
  await (await arrived).finished();
  await rested(page);

  // A mark where each of the last two lines starts says where the line before it ends.
  for (let line = 0; line < 2; line++) {
    await page.keyboard.press("Home");
    await page.keyboard.insertText("|");
    await page.keyboard.press("ArrowUp");
  }
  const lines = (await texts(page)).find((text) => text.includes("|")).split("|");
  expect(lines).toHaveLength(3);
  const ends = WORDS.map((_, word) => WORDS.slice(0, word).join("").length);
  let written = 0;
  for (const line of lines.slice(0, 2)) {
    written += line.length;
    expect(ends, `the line that ends in "${line.slice(-8)}"`).toContain(written % SENTENCE.length);
  }
});

test("a word list that cannot be fetched is done without, and not asked for again", async ({ page }) => {
  await page.route("**/words/*", (route) => route.abort());
  const asked = lists(page);
  await ready(page);
  await page.keyboard.insertText(SENTENCE);
  await expect.poll(() => asked).toEqual(["thai"]);
  await page.keyboard.insertText(SENTENCE);
  await rested(page);
  expect(asked).toEqual(["thai"]);
  expect((await texts(page)).join("")).toContain(SENTENCE + SENTENCE);
});

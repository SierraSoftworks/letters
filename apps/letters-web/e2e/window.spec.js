// What the page gives the window beyond keys and text: menus, the person's own words, and other writers' edits.

import { expect, test } from "@playwright/test";

import { ready, rested, texts } from "./page.js";

/** Where the caret is on the page, which is where the hidden input is kept. */
const caret = (page) => page.locator("textarea").boundingBox();

test.beforeEach(({ page }) => ready(page));

test("the secondary button on a misspelled word offers words to mend it with", async ({ page }) => {
  await page.keyboard.type("The quik ");
  await rested(page);
  const at = await caret(page);
  await page.mouse.click(at.x - 20, at.y + at.height / 2, { button: "right" });
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await texts(page)).at(-1)).toMatch(/^The (?!quik)qu\w+ $/);
});

test("a word added from the Menu key's menu is kept in this browser", async ({ page }) => {
  await page.keyboard.type("Zorvex ");
  await rested(page);
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  await page.locator("textarea").dispatchEvent("contextmenu");
  // The person's own dictionary is the last thing the menu offers.
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate(() => localStorage.getItem("letters.words"))).toBe("Zorvex");
  expect((await texts(page)).at(-1)).toBe("Zorvex ");
});

test("two pages on one document take in each other's edits", async ({ page, context }) => {
  const other = await context.newPage();
  await ready(other);
  const json = await page.evaluate(() => window.letters.json());
  for (const [replica, typed] of [[page, "from the first"], [other, "from the second"]]) {
    await replica.bringToFront();
    await replica.evaluate((json) => window.letters.open(json), json);
    await replica.keyboard.type(typed);
  }
  const history = (replica) => replica.evaluate(() => Array.from(window.letters.history()));
  const merge = (replica, history) => replica.evaluate((history) => window.letters.merge(new Uint8Array(history)), history);
  await merge(page, await history(other));
  await merge(other, await history(page));

  const merged = await texts(page);
  expect(merged.at(-1)).toContain("from the first");
  expect(merged.at(-1)).toContain("from the second");
  expect(await texts(other)).toEqual(merged);
});

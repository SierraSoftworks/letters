// What the first frame does without: every font but the first, the dictionaries, and the maths faces of a
// document that has no maths.

import { expect, test } from "@playwright/test";

import { rested, texts } from "./page.js";

/** Holds back every request for `files`, as a slow connection would. Returns what lets them through. */
const held = async (page, files) => {
  let arrive;
  const wait = new Promise((release) => (arrive = release));
  await page.route(files, async (route) => {
    await wait;
    await route.continue();
  });
  return arrive;
};

const painted = (page) => page.waitForFunction(() => window.letters?.frames > 0);

/** Has the page carry `document`, as a server would send it for that document. */
const carrying = (page, document) =>
  page.route("/", async (route) => {
    const response = await route.fetch();
    const html = await response.text();
    const carried = /(<script type="application\/json" id="letters-document">)[^<]*(<\/script>)/;
    expect(html).toMatch(carried);
    await route.fulfill({ response, body: html.replace(carried, (_, open, close) => open + JSON.stringify(document) + close) });
  });

test("the first frame waits for the first font alone, and text is set again in its own face when that arrives", async ({ page }) => {
  const runs = [{ text: "Upright, " }, { text: "slanted", marks: ["italic"] }, { text: " and " }, { text: "typed", marks: ["code"] }];
  await carrying(page, { format: 1, language: "en", blocks: [{ id: "p1", type: "paragraph", content: runs }] });
  const arrive = await held(page, /\/fonts\/(?!Inter\.ttf)/);
  await page.goto("/");
  await painted(page);
  expect(await page.evaluate(() => window.letters.fonts)).toBe(false);
  // Without the keyboard there is no caret, so the canvas changes only when its text does.
  await page.locator("textarea").blur();
  await rested(page);
  const canvas = () => page.evaluate(() => document.querySelector("canvas").toDataURL());
  const before = await canvas();

  arrive();
  await page.waitForFunction(() => window.letters.fonts);
  await rested(page);
  expect(await canvas()).not.toBe(before);
  expect((await texts(page))[0]).toBe("Upright, slanted and typed");
});

test("the first frame does not wait for dictionaries, and misspellings are marked when they arrive", async ({ page }) => {
  const arrive = await held(page, /\/(dictionaries|maths)\//);
  await page.goto("/");
  await painted(page);
  expect(await page.evaluate(() => window.letters.spelling)).toBe(false);
  await page.keyboard.type("The quik ");

  arrive();
  await page.waitForFunction(() => window.letters.spelling);
  await rested(page);
  const at = await page.locator("textarea").boundingBox();
  await page.mouse.click(at.x - 20, at.y + at.height / 2, { button: "right" });
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await texts(page)).at(-1)).toMatch(/^The (?!quik)qu\w+ $/);
});

test("a document with maths is not shown before its maths faces are here", async ({ page }) => {
  // The page as a server would send it for a document that holds a formula.
  const formula = { format: 1, language: "en", blocks: [{ id: "f1", type: "formula", content: [{ text: "x^2" }] }] };
  await carrying(page, formula);
  const arrive = await held(page, /\/maths\//);
  const asked = page.waitForRequest(/\/maths\/KaTeX_Main-Regular\.ttf$/);
  await page.goto("/");
  await asked;
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => window.letters.frames)).toBeUndefined();

  arrive();
  await painted(page);
  expect(await page.evaluate(() => JSON.parse(window.letters.json()).blocks.map((block) => block.type))).toEqual(["formula"]);
});

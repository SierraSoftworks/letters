// What the first frame does without: every font but the first, the dictionaries, and the maths engine and
// faces of a document that has no maths.

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
  // Text in Latin script waits for the stand-in for the first font, a third of the whole face.
  const arrive = await held(page, /\/fonts\/(?!Inter-Latin\.ttf)/);
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

test("a document the stand-in for the first font cannot write waits for the whole face, and has its text from the first frame", async ({ page }) => {
  await carrying(page, { format: 1, language: "el", blocks: [{ id: "p1", type: "paragraph", content: [{ text: "Πρακτικά" }] }] });
  const asked = [];
  page.on("request", (request) => asked.push(new URL(request.url()).pathname));
  const arrive = await held(page, /\/fonts\/(?!Inter\.ttf)/);
  await page.goto("/");
  await painted(page);
  expect(await page.evaluate(() => window.letters.fonts)).toBe(false);
  await page.locator("textarea").blur();
  await rested(page);
  const canvas = () => page.evaluate(() => document.querySelector("canvas").toDataURL());
  const before = await canvas();

  arrive();
  await page.waitForFunction(() => window.letters.fonts);
  await rested(page);
  // Upright Greek was in its own face from the start, so the faces that came later changed nothing.
  expect(await canvas()).toBe(before);
  expect(asked.filter((path) => path.endsWith("/fonts/Inter.ttf")).length).toBe(1);
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

/** A document that holds a formula in a line of text. */
const MATHS = { format: 1, language: "en", blocks: [{ id: "p1", type: "paragraph", content: [{ text: "Mass, " }, { text: "\\frac{a}{b}", marks: ["math"] }] }] };
/** The canvas as it is once it rests, with no caret on it. */
const shown = async (page) => {
  await page.locator("textarea").blur();
  await rested(page);
  return page.evaluate(() => document.querySelector("canvas").toDataURL());
};
/** The addresses the page asks for from here on, without what follows a `?`. */
const requests = (page) => {
  const asked = [];
  page.on("request", (request) => asked.push(new URL(request.url()).pathname));
  return asked;
};

for (const [what, files] of [["maths faces are", /\/maths\//], ["maths engine is", /\/maths(\.js|_bg\.wasm)/]]) {
  test(`a document with maths is not shown before its ${what} here`, async ({ page }) => {
    // The page as a server would send it for a document that holds a formula.
    const formula = { format: 1, language: "en", blocks: [{ id: "f1", type: "formula", content: [{ text: "x^2" }] }] };
    await carrying(page, formula);
    const arrive = await held(page, files);
    const asked = page.waitForRequest(files);
    await page.goto("/");
    await asked;
    await page.waitForTimeout(300);
    expect(await page.evaluate(() => window.letters.frames)).toBeUndefined();

    arrive();
    await painted(page);
    expect(await page.evaluate(() => window.letters.maths)).toBe(true);
    expect(await page.evaluate(() => JSON.parse(window.letters.json()).blocks.map((block) => block.type))).toEqual(["formula"]);
  });
}

test("a document with no maths fetches no engine, and its first formula shows as written until the engine has come", async ({ page }) => {
  const asked = requests(page);
  const arrive = await held(page, /\/maths\.js/);
  await page.goto("/");
  await page.waitForFunction(() => window.letters?.frames > 0 && window.letters.fonts && window.letters.spelling);
  await rested(page);
  expect(asked.filter((path) => /\/maths(\.js|_bg\.wasm)$/.test(path))).toEqual([]);
  expect(await page.evaluate(() => window.letters.maths)).toBe(false);

  await page.evaluate((json) => window.letters.open(json), JSON.stringify(MATHS));
  await expect.poll(() => asked.filter((path) => path.endsWith("/maths.js")).length).toBe(1);
  const written = await shown(page);

  arrive();
  await page.waitForFunction(() => window.letters.maths);
  const set = await shown(page);
  expect(set).not.toBe(written);
  expect(asked.filter((path) => path.endsWith("/maths.js")).length).toBe(1);

  // It is the formula a page shows that had the engine from the start.
  const other = await page.context().newPage();
  await carrying(other, MATHS);
  await other.goto("/");
  await other.waitForFunction(() => window.letters?.frames > 0 && window.letters.fonts && window.letters.spelling);
  expect(await other.evaluate(() => window.letters.maths)).toBe(true);
  expect(await shown(other)).toBe(set);
});

test("an engine that cannot be fetched leaves the formula as written, and is asked for again after a while", async ({ page }) => {
  const asked = requests(page);
  await page.route("**/maths.js", (route) => route.abort());
  await page.goto("/");
  await painted(page);
  await page.evaluate((json) => window.letters.open(json), JSON.stringify(MATHS));
  await expect.poll(() => asked.filter((path) => path.endsWith("/maths.js")).length).toBe(1);
  const written = await shown(page);
  expect(await page.evaluate(() => window.letters.maths)).toBe(false);

  // The page is written on as before. Not every key asks again: the next try waits some seconds, and
  // then for the formula's text to be set again.
  await page.mouse.click(400, 300);
  await page.keyboard.press("ControlOrMeta+Home");
  await page.keyboard.type("Of ");
  await expect.poll(async () => (await texts(page))[0]).toMatch(/^Of Mass, /);
  await rested(page);
  expect(asked.filter((path) => path.endsWith("/maths.js")).length).toBe(1);
  await page.unroute("**/maths.js");
  await page.waitForTimeout(5200);
  await page.keyboard.type("all ");
  await page.waitForFunction(() => window.letters.maths);
  expect(await shown(page)).not.toBe(written);
  expect((await texts(page))[0]).toMatch(/^Of all Mass, /);
});

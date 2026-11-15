// Fingers and a soft keyboard, as far as a desktop browser can play them.

import { expect, test } from "@playwright/test";

import { blocks, ready, texts } from "./page.js";

test.use({ hasTouch: true });

/** Which paragraph is at `y` on the screen: taps there, types `mark` and finds it. */
const under = async (page, y, mark) => {
  await page.touchscreen.tap(550, y);
  await page.keyboard.type(mark);
  await expect.poll(async () => (await texts(page)).filter((text) => text.includes(mark))).toHaveLength(1);
  return (await texts(page)).findIndex((text) => text.includes(mark));
};

/** Opens a document long enough to scroll, which shows its end. */
const long = (page) =>
  page.evaluate(() => {
    const document = JSON.parse(window.letters.json());
    const paragraph = document.blocks.find((block) => block.type === "paragraph");
    const text = (at) => [{ ...paragraph.content[0], text: `Paragraph ${at}` }];
    document.blocks = Array.from({ length: 120 }, (_, at) => ({ ...paragraph, id: `long${at}`, content: text(at) }));
    window.letters.open(JSON.stringify(document));
  });

/** One finger down at `from`, moved by `steps` of `dy` pixels with `pause` ms between them, and lifted. */
const drag = async (page, from, dy, steps, pause) => {
  const touch = await page.context().newCDPSession(page);
  const point = (at) => [{ x: from.x, y: from.y + dy * at, id: 1 }];
  await touch.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: point(0) });
  for (let at = 1; at <= steps; at++) {
    await page.waitForTimeout(pause);
    await touch.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: point(at) });
  }
  return () => touch.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
};

test.beforeEach(async ({ page }) => {
  await ready(page);
  await long(page);
  await page.keyboard.press("ControlOrMeta+Home");
});

test("a finger drags the document, and it stays where the finger leaves it", async ({ page }) => {
  const before = await under(page, 400, "!");
  const lift = await drag(page, { x: 550, y: 500 }, -40, 5, 30);
  await page.waitForTimeout(200);
  await lift();
  // 200 pixels up the screen, less the few before it counted as a drag: some six paragraphs.
  const after = await under(page, 400, "?");
  expect(after - before).toBeGreaterThanOrEqual(4);
  expect(after - before).toBeLessThanOrEqual(9);
  await page.waitForTimeout(200);
  expect(await under(page, 400, "+")).toBe(after);
});

test("a flick carries the document on after the finger has gone", async ({ page }) => {
  const before = await under(page, 400, "!");
  const lift = await drag(page, { x: 550, y: 500 }, -60, 4, 8);
  await lift();
  // Long enough for the glide to come to rest: a finger put down sooner would only stop it.
  await page.waitForTimeout(2500);
  // The finger went 240 pixels, some nine paragraphs.
  expect((await under(page, 400, "?")) - before).toBeGreaterThan(15);
});

/** The handles on show, as the middles of their dots from left to right. */
const handles = (page) =>
  page.evaluate(() =>
    [...document.querySelectorAll("[data-letters=handle]:not([hidden])")]
      .map((handle) => handle.getBoundingClientRect())
      .map((box) => ({ x: box.x + box.width / 2, y: box.y + 8 }))
      .sort((a, b) => a.x - b.x),
  );

/** What is selected in the caret's paragraph, as the hidden input holds it. */
const selected = (page) =>
  page.locator("textarea").evaluate((input) => input.value.slice(input.selectionStart, input.selectionEnd));

/** One finger down at `from`, slid `dx` pixels along the line, and lifted. */
const slide = async (page, from, dx) => {
  const touch = await page.context().newCDPSession(page);
  const point = (at) => [{ x: from.x + (dx * at) / 4, y: from.y, id: 1 }];
  await touch.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: point(0) });
  for (let at = 1; at <= 4; at++) {
    await touch.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: point(at) });
  }
  await touch.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
};

test("a selection a finger made has a handle at each end, and a finger on one moves that end", async ({ page }) => {
  // Two taps on a word select it.
  await page.touchscreen.tap(360, 300);
  await page.touchscreen.tap(360, 300);
  await expect.poll(() => selected(page)).toBe("Paragraph");
  const [start, end] = await handles(page);
  expect(end.x - start.x).toBeGreaterThan(40);
  expect(end.y).toBeCloseTo(start.y, 0);

  await slide(page, end, 30);
  await expect.poll(() => selected(page)).toMatch(/^Paragraph \d/);
  const longer = await selected(page);
  await slide(page, (await handles(page))[0], 25);
  await expect.poll(async () => longer.endsWith(await selected(page)) && (await selected(page)).length < longer.length).toBe(true);
  // The handles went with their ends.
  const [from, to] = await handles(page);
  expect(from.x).toBeGreaterThan(start.x);
  expect(to.x).toBeGreaterThan(end.x);

  // A caret has none, and neither has a selection a mouse makes.
  await page.touchscreen.tap(550, 300);
  await expect.poll(() => handles(page)).toHaveLength(0);
  await page.mouse.dblclick(360, 300);
  await expect.poll(() => selected(page)).toBe("Paragraph");
  expect(await handles(page)).toHaveLength(0);
});

test("a soft keyboard's backspace and Enter arrive although no key says so", async ({ page }) => {
  await page.keyboard.press("ControlOrMeta+End");
  const input = page.locator("textarea");
  const last = async () => (await blocks(page)).at(-1).text;
  const length = (await last()).length;
  // The keyboard deletes the letter before the caret, and reports only that the input changed.
  await input.evaluate((input) => {
    input.value = input.value.slice(0, -1);
    input.dispatchEvent(new InputEvent("input", { inputType: "deleteContentBackward" }));
  });
  await expect.poll(async () => (await last()).length).toBe(length - 1);
  await expect(input).toHaveValue(` ${await last()}`);
  await input.evaluate((input) => {
    input.value += "\n";
    input.dispatchEvent(new InputEvent("input", { inputType: "insertLineBreak" }));
  });
  await expect.poll(async () => (await blocks(page)).length).toBe(121);
});

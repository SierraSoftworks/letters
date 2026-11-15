// The smoke test: the page boots to a painted document, and the keyboard reaches it through the hidden input.

import { expect, test } from "@playwright/test";

import { blocks, frames, ready, texts } from "./page.js";

/** How many different colours the canvas shows, from a sample of its pixels. */
const colours = (page) =>
  page.evaluate(() => {
    const canvas = document.querySelector("canvas");
    const pixels = new Uint32Array(canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data.buffer);
    return new Set(pixels.filter((_, at) => at % 7 === 0)).size;
  });

/** Raises a composition event on the hidden input, as an input method would. */
const compose = (page, type, text) =>
  page.locator("textarea").evaluate(
    (input, [type, text]) => {
      if (type === "input") input.value = ` ${text}`;
      const event = type === "input" ? new InputEvent(type, { data: text, isComposing: true }) : new CompositionEvent(type, { data: text });
      input.dispatchEvent(event);
    },
    [type, text],
  );

test.beforeEach(({ page }) => ready(page));

test("boots to a painted document that fills the page", async ({ page }) => {
  const size = await page.evaluate(() => {
    const canvas = document.querySelector("canvas");
    return [canvas.width / devicePixelRatio, canvas.height / devicePixelRatio];
  });
  expect(size).toEqual([page.viewportSize().width, page.viewportSize().height]);
  expect(await texts(page)).toContain("Letters in the browser");
  // Paper, ink and the grey steps between them where glyphs are smoothed.
  expect(await colours(page)).toBeGreaterThan(16);
});

test("a page where nothing changes paints only its caret's blink", async ({ page }) => {
  // Long enough for the first frame's fade and the spelling check to be over.
  await page.waitForTimeout(500);
  const before = await frames(page);
  await page.waitForTimeout(1000);
  expect((await frames(page)) - before).toBeLessThan(10);
});

test("typing reaches the document and the canvas", async ({ page }) => {
  const [before, painted] = [await texts(page), await frames(page)];
  await page.keyboard.type("Typed in a browser");
  await expect.poll(() => texts(page)).toEqual([...before.slice(0, -1), "Typed in a browser"]);
  await expect.poll(() => frames(page)).toBeGreaterThan(painted);
  // The hidden input passes text on, and then holds what the document does around the caret.
  await expect(page.locator("textarea")).toHaveValue(" Typed in a browser");
});

test("editing keys and Markdown shorthand work as they do on the desktop", async ({ page }) => {
  await page.keyboard.type("Firsst");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Backspace");
  await page.keyboard.type("t");
  await page.keyboard.press("Enter");
  await page.keyboard.type("## Second");
  await expect.poll(() => blocks(page)).toEqual(expect.arrayContaining([{ type: "paragraph", text: "First" }, { type: "heading", text: "Second" }]));
});

test("shortcuts reach the editor in place of the browser", async ({ page }) => {
  await page.keyboard.type("kept");
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type("replaced");
  await expect.poll(() => texts(page)).toEqual(["replaced"]);
  await page.keyboard.press("ControlOrMeta+Z");
  await expect.poll(() => texts(page)).toContain("kept");
});

test("copy and paste go through the browser's clipboard events", async ({ page }) => {
  await page.keyboard.type("twice ");
  await page.keyboard.press("Shift+Home");
  await page.keyboard.press("ControlOrMeta+C");
  await page.keyboard.press("End");
  await page.keyboard.press("ControlOrMeta+V");
  await expect.poll(() => texts(page)).toContain("twice twice ");
});

test("a composition shows as it is composed and commits its text", async ({ page }) => {
  await compose(page, "compositionstart", "");
  await compose(page, "input", "に");
  await expect.poll(() => texts(page)).toContain("に");
  await compose(page, "input", "にほん");
  await compose(page, "compositionend", "日本");
  await expect.poll(() => texts(page)).toContain("日本");
  await expect(page.locator("textarea")).toHaveValue(" 日本");
});

test("a press on the canvas gives the keyboard back to the document", async ({ page }) => {
  await page.locator("textarea").blur();
  await page.keyboard.type("lost");
  const { width, height } = page.viewportSize();
  await page.mouse.click(width / 2, height - 40);
  await page.keyboard.type("found");
  await expect.poll(async () => (await texts(page)).join("")).toContain("found");
  expect((await texts(page)).join("")).not.toContain("lost");
});

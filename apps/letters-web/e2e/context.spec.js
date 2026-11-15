// The menu about what is under the pointer, where it has to go through the browser: the clipboard outside a
// clipboard event.

import { expect, test } from "@playwright/test";

import { blocks, ready, rested, texts } from "./page.js";

/** Asks for the menu where the caret is, as the secondary button there does. */
const ask = async (page) => {
  // The mirror follows the window a moment later: a menu that was closed must be gone from it first.
  await expect(page.getByRole("menu", { name: "Context menu" })).not.toBeAttached();
  const at = await page.locator("textarea").boundingBox();
  await page.mouse.click(at.x - 12, at.y + at.height / 2, { button: "right" });
  await expect(page.getByRole("menu", { name: "Context menu" })).toBeAttached();
};

/** Presses a control of the menu where the window draws it, as a pointer does. */
const press = async (page, role, name) => {
  // A row of the list is named with its key, where it has one.
  const box = await page.getByRole(role, { name, exact: role === "button" }).first().evaluate((element) => {
    const { x, y, width, height } = element.getBoundingClientRect();
    return { x, y, width, height };
  });
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
};

/** The kinds of thing on the clipboard, and the size of the first picture among them. */
const held = (page) =>
  page.evaluate(async () => {
    const [item] = await navigator.clipboard.read();
    if (!item) return { types: [] };
    const picture = item.types.includes("image/png") ? await createImageBitmap(await item.getType("image/png")) : null;
    return { types: [...item.types], width: picture?.width ?? 0, height: picture?.height ?? 0 };
  });

test.beforeEach(async ({ page, context, browserName }) => {
  // A headless Chromium refuses what a press allows in a window. Firefox knows no such permission.
  if (browserName === "chromium") await context.grantPermissions(["clipboard-write", "clipboard-read"]);
  await ready(page);
});

test("a code block is copied as a picture of it", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "only Chromium lets a test read the clipboard back");
  await page.keyboard.type("```");
  await page.keyboard.press("Enter");
  await page.keyboard.type("let x = 1;");
  await ask(page);
  await press(page, "menuitem", "Copy as picture");
  await expect.poll(async () => (await held(page)).types).toEqual(["image/png"]);
  const picture = await held(page);
  expect(picture.width).toBeGreaterThan(400);
  expect(picture.height).toBeGreaterThan(40);
  // Pasted back, the picture is a figure.
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.press("ArrowDown");
  await ask(page);
  await press(page, "button", "Paste");
  await expect.poll(async () => (await blocks(page)).some((block) => block.type === "figure")).toBe(true);
});

test("the menu cuts, copies and pastes with no clipboard event to carry them", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "only Chromium lets a test grant the clipboard");
  await page.keyboard.type("carried twice");
  await page.keyboard.press("ControlOrMeta+Shift+ArrowLeft");
  await rested(page);
  const at = await page.locator("textarea").boundingBox();
  await page.mouse.click(at.x + 12, at.y + at.height / 2, { button: "right" });
  await press(page, "button", "Cut");
  await expect.poll(async () => (await texts(page)).at(-1)).toBe("carried ");
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe("twice");
  for (const [role, name] of [["button", "Paste"], ["menuitem", "Paste as plain text"]]) {
    await ask(page);
    await press(page, role, name);
    await rested(page);
  }
  // Asking for the menu moves the caret, so where each paste lands is not the point.
  await expect.poll(async () => (await texts(page)).at(-1).length).toBe("carried twicetwice".length);
});

test("F7 goes to a misspelled word, and the number of a correction mends it", async ({ page }) => {
  await page.keyboard.type("The quik fox. ");
  await rested(page);
  await page.keyboard.press("F7");
  await expect(page.getByRole("menu", { name: "Context menu" })).toBeAttached();
  // A browser keeps the number with Ctrl for its tabs, so the number alone does it too.
  await page.keyboard.press("2");
  await expect.poll(async () => (await texts(page)).at(-1)).toBe("The quick fox. ");
});

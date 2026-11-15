// The mirror: the window as HTML beside the canvas, which is what assistive technology reads and presses.

import { expect, test } from "@playwright/test";

import { blocks, ready, rested, texts } from "./page.js";

/** Activates an element the way assistive technology does: with a click that no pointer made. */
const press = (element) => element.dispatchEvent("click");

test.beforeEach(({ page }) => ready(page));

test("the document reads as headings and paragraphs, and follows what is typed", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Letters in the browser", level: 1 })).toBeAttached();
  await page.keyboard.type("Read aloud");
  await expect(page.getByRole("document").getByRole("paragraph").filter({ hasText: "Read aloud" })).toBeAttached();
  await page.keyboard.press("Enter");
  await page.keyboard.type("## And a section");
  await expect(page.getByRole("heading", { name: "And a section", level: 2 })).toBeAttached();
  // The canvas is not offered to a reader, who has the mirror.
  await expect(page.locator("canvas")).toHaveAttribute("aria-hidden", "true");
});

test("a control pressed in the mirror is pressed in the window, and the keyboard stays with the document", async ({ page }) => {
  await page.keyboard.type("typed");
  await expect.poll(() => texts(page)).toContain("typed");
  await press(page.getByRole("button", { name: "Undo" }));
  await expect.poll(() => texts(page)).not.toContain("typed");
  await page.keyboard.type("again");
  await expect.poll(() => texts(page)).toContain("again");
});

test("activating a block puts the caret at its start", async ({ page }) => {
  await press(page.getByRole("heading", { name: "Letters in the browser" }));
  await page.keyboard.type("Now: ");
  await expect.poll(() => blocks(page)).toContainEqual({ type: "heading", text: "Now: Letters in the browser" });
});

test("a menu that opens is in the mirror, and its items can be chosen there", async ({ page }) => {
  await page.keyboard.type("The quik ");
  await rested(page);
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  await page.locator("textarea").dispatchEvent("contextmenu");
  await expect(page.getByRole("menu", { name: "Context menu" })).toBeAttached();
  await press(page.getByRole("menuitem", { name: "quick" }));
  await expect.poll(async () => (await texts(page)).at(-1)).toBe("The quick ");
  await expect(page.getByRole("menu")).toHaveCount(0);
});

test("the hidden input says which control has the keyboard inside the window", async ({ page }) => {
  const input = page.locator("textarea");
  await expect(input).not.toHaveAttribute("aria-activedescendant");
  await press(page.getByRole("button", { name: "Font" }));
  await expect(page.getByRole("button", { name: "Font" })).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("ArrowDown");
  await expect(input).toHaveAttribute("aria-activedescendant", /^letters-node-\d+$/);
  const focused = await input.getAttribute("aria-activedescendant");
  await expect(page.locator(`[id="${focused}"]`)).toBeAttached();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "Font" })).toHaveAttribute("aria-expanded", "false");
});

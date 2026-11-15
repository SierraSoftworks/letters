// Find and replace, tried on the line a test types under the page the browser opens with: the bar opens on its
// keys in place of the browser's own search and history, what is typed
// in it is found on the page, and what is found is replaced.

import { expect, test } from "@playwright/test";

import { ready, texts } from "./page.js";

const bar = (page) => page.getByRole("dialog", { name: "Find and replace" });
const field = (page, name) => page.getByRole("textbox", { name, exact: true });
/** Presses a control of the window, where the mirror says it is. */
const press = async (page, control) => {
  const box = await control.first().boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
};
/** What the bar says of what it found. */
const said = (page) => bar(page).getByRole("status");

test.beforeEach(async ({ page }) => {
  await ready(page);
  await page.keyboard.type("One two, two three");
});

test("the keys open the bar and are kept from the browser", async ({ page }) => {
  await page.evaluate(() => {
    window.kept = {};
    window.addEventListener("keydown", (event) => (window.kept[event.key.toLowerCase()] = event.defaultPrevented));
  });
  await page.keyboard.press("ControlOrMeta+f");
  await expect(bar(page)).toBeAttached();
  await expect(field(page, "Find")).toBeAttached();
  await expect(field(page, "Replace with")).not.toBeAttached();
  await page.keyboard.press("ControlOrMeta+h");
  await expect(field(page, "Replace with")).toBeAttached();
  expect(await page.evaluate(() => [window.kept.f, window.kept.h])).toEqual([true, true]);

  await page.keyboard.press("Escape");
  await expect(bar(page)).not.toBeAttached();
  await page.keyboard.type("!");
  expect((await texts(page)).at(-1)).toContain("!");
});

test("what is typed in the bar is found and stepped through, and the page is not typed on", async ({ page }) => {
  await page.keyboard.press("ControlOrMeta+f");
  await page.keyboard.type("two");
  await expect(said(page)).toHaveText("1 of 2");
  await page.keyboard.press("Enter");
  await expect(said(page)).toHaveText("2 of 2");
  await page.keyboard.press("Shift+Enter");
  await expect(said(page)).toHaveText("1 of 2");
  expect((await texts(page)).at(-1)).toBe("One two, two three");
});

test("what is found is replaced at one place, and then at every place", async ({ page }) => {
  await page.keyboard.press("ControlOrMeta+h");
  await page.keyboard.type("two");
  await expect(field(page, "Replace with")).toBeAttached();
  await press(page, field(page, "Replace with"));
  await page.keyboard.type("2");
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await texts(page)).at(-1)).toBe("One 2, two three");
  await press(page, page.getByRole("button", { name: "Regular expression", exact: true }));
  await press(page, field(page, "Find"));
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("t(\\w+)");
  await press(page, field(page, "Replace with"));
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("T$1");
  await press(page, page.getByRole("button", { name: "Replace all", exact: true }));
  await expect.poll(async () => (await texts(page)).at(-1)).toBe("One 2, Two Three");
});

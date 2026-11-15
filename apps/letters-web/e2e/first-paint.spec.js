// The page as a server sends it: the document as HTML to read at once, which the editor then takes over.

import { expect, test } from "@playwright/test";

import { texts } from "./page.js";

test("the document can be read before the editor arrives, and the editor opens the same one", async ({ page }) => {
  // The editor's code is held back, as a slow connection would hold it.
  let arrive;
  const held = new Promise((release) => (arrive = release));
  await page.route("**/letters_bg.wasm", async (route) => {
    await held;
    await route.continue();
  });
  await page.goto("/");
  const first = page.frameLocator("#letters-first-paint");
  await expect(first.getByRole("heading", { name: "Letters in the browser" })).toBeVisible();
  await expect(first.getByText("Type to try it.")).toBeVisible();
  // Until the editor starts, the page's `letters` is only the canvas of that name.
  expect(await page.evaluate(() => window.letters.frames)).toBeUndefined();

  arrive();
  await page.waitForFunction(() => window.letters?.frames > 0);
  // The canvas has the document now, so the HTML that stood in for it is gone.
  await expect(page.locator("#letters-first-paint")).toHaveCount(0);
  expect(await texts(page)).toContain("Letters in the browser");
  const ids = await page.evaluate(() => JSON.parse(window.letters.json()).blocks.map((block) => block.id));
  expect(ids).toEqual(["welcome1", "welcome2", "welcome3"]);
});

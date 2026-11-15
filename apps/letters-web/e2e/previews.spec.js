// Link previews: a page may not read another site's pages, so a preview's fetch waits for a server.

import { expect, test } from "@playwright/test";

import { blocks, ready, rested } from "./page.js";

test("a link preview that is put in says why it was not fetched, and is asked for no longer", async ({ page }) => {
  await ready(page);
  await page.keyboard.press("ControlOrMeta+Shift+p");
  await page.keyboard.type("Link preview");
  await rested(page);
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog", { name: "Link preview" })).toBeAttached();
  await page.keyboard.type("https://example.com/article");
  await page.keyboard.press("Enter");

  // The preview is in the document with its address, and its control offers another try with the reason.
  await expect.poll(async () => (await blocks(page)).map((block) => block.type)).toContain("link_preview");
  const again = page.getByRole("button", { name: "Try again" });
  await expect(again).toHaveAccessibleDescription(/Not fetched: a preview cannot be fetched in the browser/);
  await expect(page.getByRole("button", { name: /Fetching/ })).not.toBeAttached();

  // Another try is answered as the first was, and never left waiting.
  await again.dispatchEvent("click");
  await rested(page);
  await expect(again).toHaveAccessibleDescription(/Not fetched/);
  await expect(page.getByRole("button", { name: /Fetching/ })).not.toBeAttached();
});

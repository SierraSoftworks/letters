// Link previews: a page may not read another site's pages, so a preview's fetch waits for a server.

import { expect, test } from "@playwright/test";

import { blocks, ready, rested } from "./page.js";

test("a link preview that is put in shows its address, and offers no fetch that could not succeed", async ({ page }) => {
  await ready(page);
  await page.keyboard.press("ControlOrMeta+Shift+p");
  await page.keyboard.type("Link preview");
  await rested(page);
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog", { name: "Link preview" })).toBeAttached();
  await page.keyboard.type("https://example.com/article");
  await page.keyboard.press("Enter");

  await expect.poll(async () => (await blocks(page)).map((block) => block.type)).toContain("link_preview");
  await expect(page.getByRole("link", { name: "https://example.com/article" })).toBeAttached();
  // Nothing waits for an answer that never comes, and nothing asks for one.
  await rested(page);
  await expect(page.getByRole("button", { name: /Fetch|Try again/ })).not.toBeAttached();
});

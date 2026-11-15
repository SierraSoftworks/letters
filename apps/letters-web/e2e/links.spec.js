// Links in the document, which the shell opens: the person sees the whole address first.

import { expect, test } from "@playwright/test";

import { ready, rested } from "./page.js";

/** Opens a document whose paragraph is "Visit the site", the last two words a link to `href`. */
const linked = (page, href) =>
  page.evaluate((href) => {
    const document = JSON.parse(window.letters.json());
    const paragraph = document.blocks.find((block) => block.type === "paragraph");
    paragraph.content = [{ text: "Visit " }, { text: "the site", link: new URL(href, location.href).href }];
    window.letters.open(JSON.stringify(document));
    return paragraph.content[1].link;
  }, href);

/** Follows the link, as Ctrl or Cmd with a press on it does. */
const follow = async (page) => {
  await rested(page);
  await page.keyboard.down("ControlOrMeta");
  await page.mouse.click(400, 151);
  await page.keyboard.up("ControlOrMeta");
};

test.beforeEach(({ page }) => ready(page));

test("a link is opened in another tab once the person has read its address", async ({ page }) => {
  const href = await linked(page, "/letters.js");
  await follow(page);
  await expect(page.getByText(href)).toBeAttached();
  const box = await page.getByRole("button", { name: "Open link" }).boundingBox();
  const [opened] = await Promise.all([
    page.waitForEvent("popup"),
    page.mouse.click(box.x + box.width / 2, box.y + box.height / 2),
  ]);
  await opened.waitForURL(href);
  // The page it opens cannot reach back into this one.
  expect(await opened.evaluate(() => window.opener)).toBeNull();
});

test("an address that would run something here is never opened", async ({ page }) => {
  await linked(page, "javascript:document.title='run'");
  await follow(page);
  await expect(page.getByText("This link was not opened")).toBeAttached();
  await expect(page.getByRole("button", { name: "Open link" })).not.toBeAttached();
  expect(await page.title()).not.toBe("run");
});

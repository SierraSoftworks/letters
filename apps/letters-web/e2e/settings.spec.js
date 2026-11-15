// What the person sets for themselves: kept in this browser, and followed by its other tabs.

import { expect, test } from "@playwright/test";

import { ready } from "./page.js";

/** Clicks the control with this name and role where the canvas draws it. */
const click = async (page, name, role = "button") => {
  const box = await page.getByRole(role, { name, exact: true }).first().boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
};

const kept = (page) =>
  page.evaluate(() => Object.fromEntries(["theme", "accent", "name", "language"].map((key) => [key, localStorage.getItem(`letters.${key}`)])));

const pressed = (page, name) => page.getByRole("button", { name, exact: true, pressed: true });

const open = async (page) => {
  await page.keyboard.press("ControlOrMeta+,");
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeAttached();
};

test.beforeEach(({ page }) => ready(page));

test("what the person sets is still theirs after the page is loaded again", async ({ page }) => {
  await click(page, "More");
  await click(page, "Settings", "menuitem");
  await click(page, "Dark");
  await click(page, "Green");
  await click(page, "Your name", "textbox");
  await page.keyboard.type("Ada Lovelace");
  await page.keyboard.press("Enter");
  await click(page, "Language");
  await click(page, "English (United Kingdom)", "menuitemcheckbox");
  await expect.poll(() => kept(page)).toEqual({ theme: "dark", accent: "#1f8a5b", name: "Ada Lovelace", language: "en-GB" });

  await page.reload();
  await page.waitForFunction(() => window.letters?.frames > 0);
  await open(page);
  for (const name of ["Dark", "Green"]) {
    await expect(pressed(page, name)).toBeAttached();
  }
});

test("a setting changed in one tab is followed by the others", async ({ page, context }) => {
  const other = await context.newPage();
  await ready(other);
  await open(other);
  await click(other, "Violet");

  await page.bringToFront();
  await open(page);
  await expect(pressed(page, "Violet")).toBeAttached();
});

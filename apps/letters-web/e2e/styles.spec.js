// What the window asks the browser for: a stylesheet's file out and in, somewhere to keep the person's
// default style, and a name for the document.

import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { ready } from "./page.js";

/** Clicks the control with this name where the canvas draws it. A browser opens a file chooser for a real press only. */
const click = async (page, name) => {
  const box = await page.getByRole("button", { name, exact: true }).first().boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
};

/** Takes the document's stylesheet from the open Styles panel, as the file the browser is handed. */
const exported = async (page) => {
  const [download] = await Promise.all([page.waitForEvent("download"), click(page, "Export")]);
  return { name: download.suggestedFilename(), text: await readFile(await download.path(), "utf8") };
};

/** Hands the window `text` as the stylesheet the person chose. */
const imported = async (page, text) => {
  const [chooser] = await Promise.all([page.waitForEvent("filechooser"), click(page, "Import")]);
  await chooser.setFiles({ name: "House.letstyle", mimeType: "application/json", buffer: Buffer.from(text) });
  await expect(page.getByText("House.letstyle")).toBeAttached();
};

const wider = (text) => text.replace(/"margin_left":\s*[\d.]+/, '"margin_left": 99');

test.beforeEach(async ({ page }) => {
  await ready(page);
  await click(page, "Styles");
});

test("a stylesheet goes out as a file, and one that comes in is offered before anything changes", async ({ page }) => {
  const sheet = await exported(page);
  expect(sheet.name).toMatch(/\.letstyle$/);
  expect(Object.keys(JSON.parse(sheet.text).types)).toContain("body");

  await imported(page, wider(sheet.text));
  await expect(page.getByRole("button", { name: "Apply" })).toBeAttached();
  expect((await exported(page)).text).toBe(sheet.text);
});

test("a file that is no stylesheet is refused in the panel", async ({ page }) => {
  const [chooser] = await Promise.all([page.waitForEvent("filechooser"), click(page, "Import")]);
  await chooser.setFiles({ name: "Notes.letstyle", mimeType: "application/json", buffer: Buffer.from("not a style") });
  await expect(page.getByRole("alert")).toHaveAccessibleName(/Notes\.letstyle/);
  await expect(page.getByRole("button", { name: "Apply" })).toHaveCount(0);
});

test("the default style the person keeps is still theirs after the page is loaded again", async ({ page }) => {
  const sheet = await exported(page);
  // With the built-in default, the document's style is the default.
  await expect(page.getByText("Differs from your default")).toHaveCount(0);
  await imported(page, wider(sheet.text));
  await click(page, "Save as my default");
  await expect(page.getByText("Differs from your default")).toBeAttached();

  await page.reload();
  await page.waitForFunction(() => window.letters?.frames > 0);
  await click(page, "Styles");
  await expect(page.getByText("Differs from your default")).toBeAttached();
});

test("the document takes the name the person gives it", async ({ page }) => {
  await click(page, "Close");
  await click(page, "Letters in the browser");
  await expect(page.getByRole("textbox", { name: "Document name" })).toBeAttached();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type("Rates");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Rates", exact: true })).toBeAttached();
  await expect(page).toHaveTitle("Rates");
});

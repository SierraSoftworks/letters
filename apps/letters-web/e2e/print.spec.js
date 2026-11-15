// Printing: the PDF the export writes, made in a module the page fetches when it is first asked for.

import { expect, test } from "@playwright/test";

import { ready, rested } from "./page.js";

/** Presses the control called `name`, where the mirror says it is. */
const click = async (page, name) => {
  const box = await page.getByRole("button", { name, exact: true }).first().boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
};

/** The PDF handed to the browser's print dialog, as text. */
const printed = async (page) => {
  const frame = page.locator("iframe[data-letters=print]");
  await expect(frame).toBeAttached();
  return frame.evaluate(async (frame) => new TextDecoder("latin1").decode(await (await fetch(frame.src)).arrayBuffer()));
};

test.beforeEach(({ page }) => ready(page));

test("Print makes the document's PDF, with what makes it fetched only then", async ({ page }) => {
  const fetched = [];
  page.on("request", (request) => fetched.push(new URL(request.url()).pathname));
  await page.keyboard.type("Printed from a browser");
  await page.keyboard.press("ControlOrMeta+p");
  await rested(page);
  expect(fetched.filter((path) => path.includes("export"))).toEqual([]);

  await click(page, "Print");
  const pdf = await printed(page);
  expect(pdf.slice(0, 5)).toBe("%PDF-");
  expect(pdf).toContain("/Title");
  expect(fetched).toContain("/export_bg.wasm");

  // A second print is made by the module already here, and takes the place of the first.
  fetched.length = 0;
  const first = await page.locator("iframe[data-letters=print]").getAttribute("src");
  await click(page, "Print");
  await expect.poll(() => page.locator("iframe[data-letters=print]").getAttribute("src")).not.toBe(first);
  expect(await page.locator("iframe[data-letters=print]").count()).toBe(1);
  expect(fetched.filter((path) => path.includes("export"))).toEqual([]);
});

test("a print that cannot be made says so", async ({ page }) => {
  await page.route("**/export.js*", (route) => route.abort());
  await page.keyboard.press("ControlOrMeta+p");
  await rested(page);
  const said = page.waitForEvent("dialog");
  await click(page, "Print");
  const alert = await said;
  expect(alert.message()).toContain("could not be printed");
  await alert.dismiss();

  // What could not be fetched is asked for again.
  await page.unroute("**/export.js*");
  await click(page, "Print");
  expect((await printed(page)).slice(0, 5)).toBe("%PDF-");
});

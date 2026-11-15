// The page a server sends for a document it keeps: the editor opens it as the server says.

import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { entry, rested, texts } from "./page.js";
import { opened, serve } from "./server.js";

const TRUST = `/api/v1/documents/${opened().id}/trust`;
const bar = (page) => page.getByText(/^Safe mode:/);

test("the server's page opens its document as the server says, under a policy that lets no inline script run", async ({ page }) => {
  const said = opened({ style: { format: 1, page: { margin_top: 99 } } });
  await serve(page, said);
  await expect(page).toHaveTitle("Minutes");
  expect(await texts(page)).toContain("Letters in the browser");
  await expect(bar(page)).not.toBeAttached();

  // What is kept from here is the server's document: the same one, in its own style.
  await page.keyboard.type("Written here.");
  const kept = page.waitForEvent("download");
  await page.keyboard.press("ControlOrMeta+s");
  const file = await kept;
  expect(file.suggestedFilename()).toBe("Minutes.let");
  const zip = await readFile(await file.path());
  expect(JSON.parse(entry(zip, "manifest.json"))).toMatchObject({ id: said.id, created: said.created });
  expect(JSON.parse(entry(zip, "style.json")).page.margin_top).toBe(99);

  expect(await page.evaluate(() => window.blocked)).toEqual([]);
});

const answers = [
  { name: "agrees", answer: { status: 204 }, held: false, told: null },
  {
    name: "refuses",
    answer: { status: 403, body: { code: "forbidden", message: "Only its owner may trust this document." } },
    held: true,
    told: "Only its owner may trust this document.",
  },
  { name: "cannot be read", answer: { status: 502, body: "<html>Bad gateway</html>", type: "text/html" }, held: true, told: /502/ },
];
for (const { name, answer, held, told } of answers) {
  test(`a document the server holds in safe mode is trusted through the server, which ${name}`, async ({ page }) => {
    const said = [];
    page.on("dialog", (dialog) => (said.push(dialog.message()), dialog.accept()));
    const asked = await serve(page, opened({ trust: "safe" }), (method, path) => (path === TRUST ? answer : undefined));
    await expect(bar(page)).toBeAttached();

    // Where the canvas draws the button.
    const box = await page.getByRole("button", { name: "Trust this document" }).boundingBox();
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await expect.poll(() => asked.map(({ method, path }) => `${method} ${path}`)).toEqual([`POST ${TRUST}`]);
    if (told) {
      await expect.poll(() => said).toHaveLength(1);
      expect(said[0]).toMatch(told);
    }
    await rested(page);
    await expect(bar(page)).toBeAttached({ attached: held });
    expect(said).toHaveLength(told ? 1 : 0);
  });
}

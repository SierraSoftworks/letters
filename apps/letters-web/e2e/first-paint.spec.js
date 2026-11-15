// The page as a server sends it: the document as HTML to read at once under the shapes of the window's
// controls, which the editor then takes over without anything moving.

import { expect, test } from "@playwright/test";

import { texts } from "./page.js";

/** Opens the page with the editor's code held back, as a slow connection would hold it. Returns what lets it through. */
const held = async (page) => {
  let arrive;
  const wait = new Promise((release) => (arrive = release));
  await page.route("**/letters_bg.wasm", async (route) => {
    await wait;
    await route.continue();
  });
  await page.goto("/");
  await page.locator("#letters-first-paint iframe").waitFor();
  return async () => {
    arrive();
    await page.waitForFunction(() => window.letters?.frames > 0);
  };
};

const first = (page) => page.frameLocator("#letters-first-paint iframe");
const box = (element) => {
  const { x, y, width, height } = element.getBoundingClientRect();
  return { x, y, width, height };
};
/** Where the hidden input is, which is where the editor has its caret. */
const caret = (page) => page.locator("textarea").evaluate(box);

test("the document can be read before the editor arrives, and the editor opens the same one", async ({ page }) => {
  const arrive = await held(page);
  await expect(first(page).getByRole("heading", { name: "Letters in the browser" })).toBeVisible();
  await expect(first(page).getByText("Type to try it.")).toBeVisible();
  await expect(page.getByRole("status", { name: "Loading the editor" })).toBeVisible();
  // Until the editor starts, the page's `letters` is only the canvas of that name.
  expect(await page.evaluate(() => window.letters.frames)).toBeUndefined();

  await arrive();
  // The canvas has the window now, so what stood in for it is gone.
  await expect(page.locator("#letters-first-paint")).toHaveCount(0);
  expect(await texts(page)).toContain("Letters in the browser");
  const ids = await page.evaluate(() => JSON.parse(window.letters.json()).blocks.map((block) => block.id));
  expect(ids).toEqual(["welcome1", "welcome2", "welcome3"]);
});

for (const [name, width] of [["a wide window", 1280], ["a narrow window", 820], ["a phone", 390]]) {
  test(`in ${name}, each control arrives where its shape was`, async ({ page }) => {
    await page.setViewportSize({ width, height: 700 });
    const arrive = await held(page);
    const shapes = await page.locator("#letters-first-paint i").evaluateAll(
      (shapes, box) => shapes.map(eval(box)).filter((shape) => shape.width > 0),
      String(box),
    );
    expect(shapes.length).toBeGreaterThan(4);
    // No shape lies over another, as no control does.
    const across = [...shapes].sort((a, b) => a.x - b.x);
    for (const [i, shape] of across.slice(1).entries()) {
      expect(shape.x).toBeGreaterThanOrEqual(across[i].x + across[i].width);
    }

    await arrive();
    // The mirror says where the window's controls are: its placed elements that hold no other, outside the document.
    await page.locator('[id^="letters-node-"]').first().waitFor({ state: "attached" });
    const controls = await page.locator('[id^="letters-node-"]').evaluateAll((nodes, box) => {
      const placed = (node) => node.style.position === "absolute";
      const control = (node) => placed(node) && ![...node.querySelectorAll("*")].some(placed) && !node.closest('[role="document"]');
      return nodes.filter(control).map(eval(box));
    }, String(box));
    for (const shape of shapes) {
      // The mirror rounds to whole pixels. A control the bar cuts short has a shape as wide as what shows.
      const near = (a, b) => Math.abs(a - b) <= 1;
      const match = controls.find((control) => near(control.x, shape.x) && near(control.y, shape.y));
      expect(match, `a control at ${shape.x}, ${shape.y}`).toBeTruthy();
      expect(match.height).toBeCloseTo(shape.height, 0);
      expect(shape.width).toBeLessThanOrEqual(match.width + 1);
    }
  });

  test(`in ${name}, the text stays where it was read`, async ({ page }) => {
    await page.setViewportSize({ width, height: 700 });
    const arrive = await held(page);
    const frame = await page.locator("#letters-first-paint iframe").evaluate(box);
    const heading = await first(page).getByRole("heading").evaluate(box);
    const body = await first(page).getByText("Type to try it.").evaluate(box);

    await arrive();
    // The caret waits in the empty last block: at the column's edge, where the heading began, and a
    // line below the paragraph, which holds the same words on the same lines.
    const at = await caret(page);
    expect(Math.abs(at.x - (frame.x + heading.x))).toBeLessThanOrEqual(1);
    expect(at.y).toBeGreaterThan(frame.y + body.y + body.height - 2);
    expect(at.y).toBeLessThan(frame.y + body.y + body.height + 24);
  });
}

test("the editor opens where the person had read to", async ({ page }) => {
  // A window too short for the welcome document, so that there is somewhere to read on to.
  await page.setViewportSize({ width: 800, height: 160 });
  const arrive = await held(page);
  const read = await first(page).locator("html").evaluate((html) => {
    html.ownerDocument.defaultView.scrollTo(0, 60);
    return html.ownerDocument.defaultView.scrollY;
  });
  expect(read).toBe(60);
  const frame = await page.locator("#letters-first-paint iframe").evaluate(box);
  const last = await first(page).locator("article > :last-child").evaluate(box);

  await arrive();
  // The caret waits in the last block, which is where the HTML had it.
  const at = await caret(page);
  expect(at.y).toBeGreaterThan(frame.y + last.y - 3);
  expect(at.y + at.height).toBeLessThan(frame.y + last.y + last.height + 3);
});

// The document's history, as the page tells it.

import { expect, test } from "@playwright/test";

import { history, ready, rested } from "./page.js";

// A clock that is neither UTC nor whole hours from it.
test.use({ timezoneId: "Asia/Kathmandu" });

test("a version's time is told by the browser's clock", async ({ page }) => {
  await ready(page);
  /** The time on the wall, where the browser is and in UTC. */
  const clocks = () =>
    page.evaluate(() => {
      const [now, two] = [new Date(), (part) => String(part).padStart(2, "0")];
      return {
        here: `${two(now.getHours())}:${two(now.getMinutes())}`,
        utc: `${two(now.getUTCHours())}:${two(now.getUTCMinutes())}`,
      };
    });
  const before = await clocks();
  await page.keyboard.type("Written here");
  await rested(page);
  const after = await clocks();
  await history(page);

  const told = await page.getByText(/^Today \d\d:\d\d$/).textContent();
  expect([before.here, after.here].map((clock) => `Today ${clock}`)).toContain(told);
  expect(told).not.toContain(after.utc);
});

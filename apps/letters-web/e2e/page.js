// What the tests ask of the harness page.

/** Opens the page and waits for its first frame, and for the fonts and dictionaries that follow it. */
export const ready = async (page) => {
  await page.goto("/");
  await page.waitForFunction(() => window.letters?.frames > 0 && window.letters.fonts && window.letters.spelling);
};

/** The document on screen, one entry per block: its type and its text. */
export const blocks = (page) =>
  page.evaluate(() =>
    JSON.parse(window.letters.json()).blocks.map((block) => ({
      type: block.type,
      text: (block.content ?? []).map((run) => run.text).join(""),
    })),
  );

export const texts = async (page) => (await blocks(page)).map((block) => block.text);
export const frames = (page) => page.evaluate(() => window.letters.frames);

/** Waits for the frame that follows an edit, and for the spelling check that runs once it is painted. */
export const rested = (page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => setTimeout(done, 100))));

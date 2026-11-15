// What the tests ask of the harness page.

import { inflateRawSync } from "node:zlib";

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

/** Opens the document's history. Its versions are the options of its list. */
export const history = async (page) => {
  await page.keyboard.press("ControlOrMeta+Shift+p");
  await page.keyboard.type("History");
  await rested(page);
  await page.keyboard.press("Enter");
  await page.getByRole("button", { name: "Back to editing" }).waitFor({ state: "attached" });
};

/** An entry of a zip that Letters wrote, as text. `zip` is a `Buffer`. */
export const entry = (zip, name) => {
  for (let at = 0; zip.readUInt32LE(at) === 0x04034b50; ) {
    const [method, packed] = [zip.readUInt16LE(at + 8), zip.readUInt32LE(at + 18)];
    const start = at + 30 + zip.readUInt16LE(at + 26) + zip.readUInt16LE(at + 28);
    const bytes = zip.subarray(start, start + packed);
    if (zip.toString("utf8", at + 30, at + 30 + zip.readUInt16LE(at + 26)) === name) {
      return (method === 0 ? bytes : inflateRawSync(bytes)).toString("utf8");
    }
    at = start + packed;
  }
};

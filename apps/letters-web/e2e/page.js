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

/** An entry of a zip that Letters wrote, as its bytes. `zip` is a `Buffer`. */
export const file = (zip, name) => {
  for (let at = 0; zip.readUInt32LE(at) === 0x04034b50; ) {
    const [method, packed] = [zip.readUInt16LE(at + 8), zip.readUInt32LE(at + 18)];
    const start = at + 30 + zip.readUInt16LE(at + 26) + zip.readUInt16LE(at + 28);
    const bytes = zip.subarray(start, start + packed);
    if (zip.toString("utf8", at + 30, at + 30 + zip.readUInt16LE(at + 26)) === name) {
      return method === 0 ? bytes : inflateRawSync(bytes);
    }
    at = start + packed;
  }
};

/** An entry of a zip that Letters wrote, as text. */
export const entry = (zip, name) => file(zip, name)?.toString("utf8");

/** The colour every test picture is filled with, which nothing else in the window is. */
export const INK = [255, 0, 255];

/** A PNG of one colour, as the numbers of its bytes. */
export const png = (page, width = 64, height = 48) =>
  page.evaluate(
    async ([width, height, ink]) => {
      const canvas = Object.assign(document.createElement("canvas"), { width, height });
      const context = canvas.getContext("2d");
      context.fillStyle = `rgb(${ink})`;
      context.fillRect(0, 0, width, height);
      const blob = await new Promise((done) => canvas.toBlob(done, "image/png"));
      return [...new Uint8Array(await blob.arrayBuffer())];
    },
    [width, height, INK],
  );

/** Raises `kind` on what `selector` finds, carrying files: each a name, a type and its bytes. */
export const carry = (page, selector, kind, files, at = {}) =>
  page.evaluate(
    ([selector, kind, files, at]) => {
      const data = new DataTransfer();
      for (const [name, type, bytes] of files) data.items.add(new File([new Uint8Array(bytes)], name, { type }));
      const carried = kind === "paste" ? { clipboardData: data } : { dataTransfer: data, ...at };
      const Kind = kind === "paste" ? ClipboardEvent : DragEvent;
      return document.querySelector(selector).dispatchEvent(new Kind(kind, { ...carried, bubbles: true, cancelable: true }));
    },
    [selector, kind, files, at],
  );

/** How many pixels of the canvas are in the pictures' colour. */
export const inked = (page) =>
  page.evaluate((ink) => {
    const canvas = document.getElementById("letters");
    const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
    let count = 0;
    for (let at = 0; at < pixels.length; at += 4) {
      if (pixels[at] === ink[0] && pixels[at + 1] === ink[1] && pixels[at + 2] === ink[2]) count++;
    }
    return count;
  }, INK);

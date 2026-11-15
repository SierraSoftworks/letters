// Save a copy: the document again, or in another format, as a file the browser keeps for the person.

import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { blocks, entry, ready, rested } from "./page.js";

const card = (page) => page.getByRole("dialog", { name: "Save a copy" });

/** Presses the control of the card called `name`, where the mirror says it is. */
const click = async (page, name) => {
  const box = await card(page).getByRole("button", { name, exact: true }).boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
};

/** Opens the card, and sets it to a copy called `name` in `format`. */
const ask = async (page, name, format) => {
  await page.keyboard.press("ControlOrMeta+Shift+s");
  await expect(card(page)).toBeAttached();
  await page.keyboard.type(name);
  await click(page, format);
  await rested(page);
};

/** The file the browser is handed when the card is told to save: its name, and its bytes as text. */
const saved = async (page) => {
  const kept = page.waitForEvent("download");
  await page.keyboard.press("Enter");
  const file = await kept;
  return [file.suggestedFilename(), (await readFile(await file.path())).toString("latin1")];
};

/** Pastes a small picture, which becomes a figure. */
const picture = async (page) => {
  await page.evaluate(async () => {
    const canvas = Object.assign(document.createElement("canvas"), { width: 32, height: 24 });
    canvas.getContext("2d").fillRect(0, 0, 32, 24);
    const blob = await new Promise((done) => canvas.toBlob(done, "image/png"));
    const data = new DataTransfer();
    data.items.add(new File([blob], "shot.png", { type: "image/png" }));
    const paste = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true });
    document.querySelector("textarea").dispatchEvent(paste);
  });
  await expect.poll(async () => (await blocks(page)).some((block) => block.type === "figure")).toBe(true);
};

test.beforeEach(({ page }) => ready(page));

for (const [format, ending, starts, ...holds] of [
  // A document of its own is an archive that says what it is first, and brings its history.
  ["Letters", "let", "PK", "mimetypeapplication/vnd.letters.document+zip", "history/oplog"],
  ["Word", "docx", "PK", "word/document.xml"],
  ["PDF", "pdf", "%PDF-", "/Title"],
  ["Web page", "html", "<!doctype html>", "Kept from a browser"],
  ["Markdown", "md", "", "Kept from a browser"],
]) {
  test(`a copy as ${format} is a file the browser keeps`, async ({ page }) => {
    await page.keyboard.type("Kept from a browser");
    await ask(page, "Kept: a copy", format);
    const [name, file] = await saved(page);
    // The name is what was typed, less what no file can be called.
    expect(name).toBe(`Kept a copy.${ending}`);
    expect(file.startsWith(starts)).toBe(true);
    for (const held of holds) expect(file).toContain(held);
    await expect(card(page)).not.toBeAttached();
  });
}

test("a clean copy is a document of its own, without what this one once read", async ({ page }) => {
  // A passage written and taken back is in the history alone.
  await page.keyboard.insertText("Thought better of. ".repeat(12));
  await page.keyboard.press("ControlOrMeta+z");
  await page.keyboard.type("Kept words");
  const files = [];
  for (const [name, format] of [["Whole", "Letters"], ["Clean", "Clean copy"]]) {
    await ask(page, name, format);
    const [called, file] = await saved(page);
    expect(called).toBe(`${name}.let`);
    files.push(Buffer.from(file, "latin1"));
  }
  const [whole, clean] = files;
  for (const file of files) expect(entry(file, "document.json")).toContain("Kept words");
  expect(entry(clean, "history/oplog").length).toBeLessThan(entry(whole, "history/oplog").length);
  expect(JSON.parse(entry(clean, "manifest.json")).id).not.toBe(JSON.parse(entry(whole, "manifest.json")).id);
});

test("a Markdown copy brings its pictures with it", async ({ page }) => {
  await picture(page);
  await ask(page, "Shots", "Markdown");
  const [name, file] = await saved(page);
  expect(name).toBe("Shots.zip");
  expect(file.startsWith("PK")).toBe(true);
  expect(file).toContain("Shots.md");
  expect(file).toMatch(/assets\/[^/]+\.png/);
});

test("what writes a copy is fetched only when one is asked for", async ({ page }) => {
  const fetched = [];
  page.on("request", (request) => fetched.push(new URL(request.url()).pathname));
  await ask(page, "Late", "Web page");
  expect(fetched.filter((path) => path.includes("export"))).toEqual([]);
  await saved(page);
  expect(fetched).toContain("/export_bg.wasm");
});

test("a copy that cannot be written leaves its card to ask again", async ({ page }) => {
  let downloads = 0;
  page.on("download", () => downloads++);
  // The answer comes late, as it does over a network: long after the key that asked was heard.
  const late = () => new Promise((done) => setTimeout(done, 700));
  await page.route("**/export.js*", (route) => late().then(() => route.abort()));
  await ask(page, "Later", "Web page");
  const failed = page.waitForEvent("requestfailed");
  await page.keyboard.press("Enter");
  await failed;
  await rested(page);
  expect(downloads).toBe(0);
  // Nothing was pressed since, so only the answer itself can have told a screen reader why.
  await expect(card(page).getByRole("alert")).toHaveAccessibleName(/could not be fetched/, { timeout: 1000 });

  // The card was put away while the copy was made, and is back as it was to be told again.
  await page.unroute("**/export.js*");
  const [name] = await saved(page);
  expect(name).toBe("Later.html");
  await expect(card(page)).not.toBeAttached();
});

test("a copy has its formulas set whether or not the page has its maths engine", async ({ page }) => {
  // The exports set formulas themselves, so a page that could not fetch its engine still prints them.
  await page.route("**/maths.js", (route) => route.abort());
  await ready(page);
  const maths = [{ text: "Mass, " }, { text: "\\frac{a}{b}", marks: ["math"] }];
  const document = { format: 1, language: "en", blocks: [{ id: "p1", type: "paragraph", content: maths }] };
  await page.evaluate((json) => window.letters.open(json), JSON.stringify(document));
  await rested(page);
  expect(await page.evaluate(() => window.letters.maths)).toBe(false);

  await ask(page, "Maths", "PDF");
  const [, pdf] = await saved(page);
  expect(pdf).toMatch(/KaTeX_Ma(th|in)/);
  await ask(page, "Maths", "Web page");
  const [, html] = await saved(page);
  expect(html).toMatch(/class="math inline"[^>]*><svg/);
});

test.describe("a PDF of emoji and of scripts the fonts lack", () => {
  // The faces that back up the fonts are fetched as the page is built, and a build without a network has none.
  const built = existsSync(new URL("../dist/fallback/NotoColorEmoji.ttf", import.meta.url));
  test.skip(!built, "this build has no faces to back up the fonts");

  /** A PDF of a line with an emoji and Japanese in it. */
  const pdf = async (page) => {
    await page.keyboard.insertText("A smile 🙂 and 日本語のテキスト");
    await ask(page, "Faces", "PDF");
    return (await saved(page))[1];
  };

  test("carries the faces that have them", async ({ page }) => {
    const file = await pdf(page);
    for (const face of ["NotoColorEmoji", "NotoSansJP"]) expect(file).toContain(face);
  });

  test("is made without a face that cannot be fetched", async ({ page }) => {
    await page.route("**/fallback/NotoColorEmoji.ttf", (route) => route.abort());
    const file = await pdf(page);
    expect(file.startsWith("%PDF-")).toBe(true);
    expect(file).toContain("NotoSansJP");
    expect(file).not.toContain("NotoColorEmoji");
  });
});

test("a PDF of a script written without spaces ends its lines by the script's word list", async ({ page }) => {
  // Two PDFs written in the same second of the same document are the same file.
  await page.clock.setFixedTime(new Date("2026-11-15T12:00:00Z"));
  const asked = [];
  page.on("request", (request) => request.url().includes("/words/") && asked.push(request.url()));
  const pdf = async () => {
    await ready(page);
    await page.keyboard.insertText("ภาษาไทยเขียนติดกันโดยไม่เว้นวรรคระหว่างคำ".repeat(8));
    await ask(page, "Thai", "PDF");
    return (await saved(page))[1];
  };
  const listed = await pdf();
  expect(asked.map((url) => url.replace(/.*\/words\/|-.*/g, ""))).toEqual(["thai"]);

  // Without the list a line ends wherever it fills, and the PDF is made all the same.
  await page.route("**/words/*", (route) => route.abort());
  const plain = await pdf();
  expect(plain.startsWith("%PDF-")).toBe(true);
  expect(plain).not.toBe(listed);
});

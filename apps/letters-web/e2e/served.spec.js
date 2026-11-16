// The page a server sends for a document it keeps: the editor opens it as the server says.

import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { createHash } from "node:crypto";

import { INK, blocks, carry, entry, file, inked, learn, png, rested, texts } from "./page.js";
import { album, opened, picture, room, serve } from "./server.js";

const DOCUMENT = `/api/v1/documents/${opened().id}`;
const TRUST = `${DOCUMENT}/trust`;
const bar = (page) => page.getByText(/^Safe mode:/);

/** Clicks the control with this name where the canvas draws it. */
const click = async (page, name, role = "button") => {
  const box = await page.getByRole(role, { name, exact: true }).boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
};

/** Runs an entry of the command palette. */
const run = async (page, command) => {
  await page.keyboard.press("ControlOrMeta+Shift+p");
  await page.keyboard.type(command);
  await rested(page);
  await page.keyboard.press("Enter");
};

/** What the page asked of the server, a line each. A body of JSON is told too. */
const told = (asked) =>
  asked.map(({ method, path, body, type }) => [method, path.replace(DOCUMENT, ""), ...(type === "application/json" ? [body.toString()] : [])].join(" "));

/** The server's answers to a fetch: `fetch(url, asked)` for a picture's address, `preview` for a page's. */
const fetches = ({ fetch = () => undefined, preview } = {}) => (method, path, body) => {
  if (method === "POST" && path === `${DOCUMENT}/fetch/picture`) {
    const { url, asked } = JSON.parse(body);
    const bytes = fetch(url, asked);
    return bytes ? { body: bytes, type: "image/png" } : { status: 422, body: { code: "untrusted", message: "The server may not fetch this address." } };
  }
  if (method === "POST" && path === `${DOCUMENT}/fetch/preview`) return preview;
  return method === "PUT" ? { status: 204 } : undefined;
};

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

    await click(page, "Trust this document");
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

test("someone a share link alone leads in is shown no history, and keeps the document as it stands without it", async ({ page }) => {
  const files = [];
  for (const by_link of [false, true]) {
    await serve(page, opened({ by_link }));
    await expect(page.getByRole("button", { name: "History", exact: true })).toBeAttached({ attached: !by_link });
    // A passage written and taken back is in the history alone.
    await page.keyboard.type("A passage that is written and then taken back, which only the history remembers. ".repeat(4));
    await page.keyboard.press("ControlOrMeta+z");
    const kept = page.waitForEvent("download");
    await page.keyboard.press("ControlOrMeta+s");
    files.push(await readFile(await (await kept).path()));
  }
  const [whole, clean] = files;
  expect(JSON.parse(entry(whole, "manifest.json")).id).toBe(opened().id);
  expect(JSON.parse(entry(clean, "manifest.json")).id).not.toBe(opened().id);
  expect(file(clean, "history/oplog").length).toBeLessThan(file(whole, "history/oplog").length);
});

test("the pictures of the server's document come from the server, those the page never showed too", async ({ page }) => {
  // The second lies far below what the window shows.
  const [shown, below] = [picture(INK), picture([0, 128, 255], 64, 4000)];
  const kept = new Map([shown, below].map(({ name, bytes }) => [`${DOCUMENT}/${name}`, bytes]));
  const answer = (method, path) => (method === "GET" && kept.has(path) ? { body: kept.get(path), type: "image/png" } : undefined);
  const asked = await serve(page, opened(), answer, album([{ asset: shown.name }, { asset: below.name }]));
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);

  // A copy of the document leaves none of them out.
  const download = page.waitForEvent("download");
  await page.keyboard.press("ControlOrMeta+s");
  const zip = await readFile(await (await download).path());
  for (const { name, bytes } of [shown, below]) {
    expect(file(zip, name)?.equals(bytes), name).toBe(true);
  }
  // Each was asked for once, whoever wanted it.
  expect(asked.map(({ method, path }) => `${method} ${path}`).sort()).toEqual([...kept.keys()].map((path) => `GET ${path}`).sort());
});

test("a copy is not made of a document whose picture the server does not give", async ({ page }) => {
  const said = [];
  page.on("dialog", (dialog) => (said.push(dialog.message()), dialog.accept()));
  let downloads = 0;
  page.on("download", () => downloads++);
  await serve(page, opened(), undefined, album([{ asset: picture(INK).name }]));
  await page.keyboard.press("ControlOrMeta+s");
  await expect.poll(() => said).toHaveLength(1);
  expect(said[0]).toMatch(/could not be saved.*picture/);
  expect(downloads).toBe(0);
});

const keeps = [
  { name: "keeps", answer: { status: 204 }, told: 0 },
  { name: "refuses", answer: { status: 413, body: { code: "too_large", message: "A picture may be 32 MB at most." } }, told: 1 },
];
for (const { name, answer, told } of keeps) {
  test(`a picture put into the server's document is handed to the server, which ${name} it`, async ({ page }) => {
    const said = [];
    page.on("dialog", (dialog) => (said.push(dialog.message()), dialog.accept()));
    const asked = await serve(page, opened(), (method) => (method === "PUT" ? answer : undefined));
    const bytes = Buffer.from(await png(page));
    await carry(page, "textarea", "paste", [["shot.png", "image/png", [...bytes]]]);
    await expect.poll(async () => (await blocks(page)).some((block) => block.type === "figure")).toBe(true);

    // Under the name the document has for it, which is the hash of its bytes.
    const hash = createHash("sha256").update(bytes).digest("hex");
    await expect.poll(() => asked.map(({ method, path }) => `${method} ${path}`)).toEqual([`PUT ${DOCUMENT}/assets/${hash}.png`]);
    expect(asked[0].body.equals(bytes)).toBe(true);
    expect(asked[0].type).toBe("image/png");
    // The picture is on the page either way, and the person is told when it is nowhere else.
    await expect.poll(() => inked(page)).toBeGreaterThan(1000);
    await expect.poll(() => said).toEqual(told ? ["A picture may be 32 MB at most."] : []);
  });
}

const SRC = "https://pictures.example/boat.png";
/** A document whose one picture is linked, large enough to press in the middle of the page. */
const linked = album([{ src: SRC, width: 400, height: 300 }]);
/** A PDF of the document, as text. */
const pdf = async (page) => {
  await page.keyboard.press("ControlOrMeta+Shift+s");
  await expect(page.getByRole("dialog", { name: "Save a copy" })).toBeAttached();
  await page.keyboard.type("Album");
  await click(page, "PDF");
  await rested(page);
  const kept = page.waitForEvent("download");
  await page.keyboard.press("Enter");
  return (await readFile(await (await kept).path())).toString("latin1");
};
const asking = (asked) => `POST /fetch/picture ${JSON.stringify({ url: SRC, asked })}`;

test("a linked picture is fetched by the server when the document opens, if the server trusts its source", async ({ page }) => {
  const asked = await serve(page, opened(), fetches({ fetch: () => picture(INK).bytes }), linked);
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);
  expect(told(asked)).toEqual([asking(false)]);
});

test("a linked picture the server does not fetch unasked waits for the person, who may load it or keep it", async ({ page }) => {
  const shot = picture(INK);
  const asked = await serve(page, opened(), fetches({ fetch: (_, asked) => asked && shot.bytes }), linked);
  await rested(page);
  expect(await inked(page)).toBe(0);
  expect(await pdf(page)).not.toContain("/Subtype/Image");

  await page.mouse.click(600, 300);
  await run(page, "Load image once");
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);
  expect(told(asked)).toEqual([asking(false), asking(true)]);
  // A PDF shows what the window shows.
  expect(await pdf(page)).toContain("/Subtype/Image");

  // Kept in the document, it is the document's own picture, and the server's to keep.
  await run(page, "Embed image in the document");
  await expect.poll(async () => JSON.parse(await page.evaluate(() => window.letters.json())).blocks[1].asset).toBe(shot.name);
  await expect.poll(() => told(asked).at(-1)).toBe(`PUT /${shot.name}`);
});

test("a linked picture the server will not fetch just now keeps its place, and is asked for again when the server says", async ({ page }) => {
  const slow = { status: 429, headers: { "retry-after": "1" }, body: { code: "slow", message: "Too many pictures and previews at once. Wait a minute." } };
  let turns = 0;
  const answer = fetches({ fetch: () => picture(INK).bytes });
  const asked = await serve(page, opened(), (method, path, body) => (path.endsWith("/fetch/picture") && turns++ < 1 ? slow : answer(method, path, body)), linked);
  await rested(page);
  // It is not a picture that failed, and nothing is asked again before the server's time.
  await expect(page.getByText(/Not loaded/)).not.toBeAttached();
  expect(told(asked)).toEqual([asking(false)]);

  await expect.poll(() => inked(page)).toBeGreaterThan(1000);
  expect(told(asked)).toEqual([asking(false), asking(false)]);
});

test("a document in safe mode has no linked picture fetched until the person trusts it", async ({ page }) => {
  const answer = fetches({ fetch: () => picture(INK).bytes });
  const asked = await serve(page, opened({ trust: "safe" }), (method, path, body) => (path === TRUST ? { status: 204 } : answer(method, path, body)), linked);
  await rested(page);
  expect(told(asked)).toEqual([]);

  await click(page, "Trust this document");
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);
  expect(told(asked)).toEqual(["POST /trust", asking(false)]);
});

const PAGE = "https://example.com/article";
/** Puts a link preview of `PAGE` into the document. */
const previewed = async (page) => {
  await run(page, "Link preview");
  await expect(page.getByRole("dialog", { name: "Link preview" })).toBeAttached();
  await page.keyboard.type(PAGE);
  await page.keyboard.press("Enter");
};
const preview = async (page) => JSON.parse(await page.evaluate(() => window.letters.json())).blocks.find((block) => block.type === "link_preview");

test("a link preview is what the server read of the page, and its picture becomes the document's", async ({ page }) => {
  const shot = picture(INK);
  const found = { title: "An article", description: "What it is about.", picture: "https://example.com/cover.png" };
  const asked = await serve(page, opened(), fetches({ fetch: (url) => url === found.picture && shot.bytes, preview: { body: found } }));
  await previewed(page);

  await expect.poll(() => preview(page)).toMatchObject({ url: PAGE, title: "An article", description: "What it is about.", image: shot.name });
  await expect.poll(() => told(asked)).toEqual([
    `POST /fetch/preview ${JSON.stringify({ url: PAGE, asked: true })}`,
    `POST /fetch/picture ${JSON.stringify({ url: found.picture, asked: true })}`,
    `PUT /${shot.name}`,
  ]);
});

test("a link preview the server could not read says why in the server's words, and offers another try", async ({ page }) => {
  const refusal = { status: 502, body: { code: "upstream", message: "The page answered 404." } };
  await serve(page, opened(), fetches({ preview: refusal }));
  await previewed(page);

  await expect(page.getByRole("button", { name: "Try again" })).toBeAttached();
  // The preview's card says what the page says under it.
  await click(page, "Edit link preview");
  await expect(page.getByText("Not fetched: the page answered 404")).toBeAttached();
  expect(await preview(page)).toEqual(expect.objectContaining({ url: PAGE }));
});

test("what the person set elsewhere is how the page is, and what they set here is kept by the server whole", async ({ page }) => {
  const theirs = {
    name: "Countess of Lovelace",
    appearance: "dark",
    accent: [31, 138, 91],
    dictionary: ["quik"],
    default_style: { format: 1, page: { margin_top: 99 } },
  };
  const settings = (method, path) => (path !== "/api/v1/settings" ? undefined : method === "GET" ? { body: theirs } : { status: 204 });
  const asked = await serve(page, opened(), settings);
  await page.keyboard.press("ControlOrMeta+,");
  for (const name of ["Dark", "Green"]) {
    await expect(page.getByRole("button", { name, exact: true, pressed: true })).toBeAttached();
  }
  // This browser keeps a copy, which the next page starts with.
  const kept = () => page.evaluate(() => ["theme", "name", "words"].map((key) => localStorage.getItem(`letters.${key}`)));
  expect(await kept()).toEqual(["dark", "Countess of Lovelace", "quik"]);
  expect(asked).toEqual([]);

  await click(page, "Light");
  await expect.poll(() => asked.map(({ method, path }) => `${method} ${path}`)).toEqual(["PUT /api/v1/settings"]);
  expect(JSON.parse(asked[0].body)).toEqual({ ...theirs, appearance: "light" });
});

/**
 * An account that holds `words` and `sources` of pictures, and takes one or lets one go as the server does.
 * `refusal` is its answer to either instead.
 */
const WORD = "/api/v1/settings/dictionary/";
const SOURCE = "/api/v1/settings/sources/";
const account = ({ words = [], sources = [], refusal } = {}) => (method, path) => {
  if (path === "/api/v1/settings") return method === "GET" ? { body: { dictionary: words } } : { status: 204 };
  if (path === SOURCE.slice(0, -1)) return { body: sources };
  const [kept, under] = path.startsWith(WORD) ? [words, WORD] : [sources, SOURCE];
  if (!path.startsWith(under)) return undefined;
  if (refusal) return refusal;
  const named = decodeURIComponent(path.slice(under.length));
  // The server writes a source as a whole address.
  const one = kept === words || named.includes("://") ? named : `https://${named}/`;
  const at = kept.indexOf(one);
  if (method === "PUT" && at < 0) kept.push(one);
  if (method === "DELETE" && at >= 0) kept.splice(at, 1);
  return { body: kept };
};
/** What the page asked of the account, a line each, with what it named as it is written. */
const named = (asked) => told(asked).filter((line) => line.includes("/api/v1/settings/")).map(decodeURIComponent);
const words = (page) => page.evaluate(() => localStorage.getItem("letters.words"));
const opens = async (page) => {
  await page.reload();
  await page.waitForFunction(() => window.letters?.frames > 0 && window.letters.spelling);
};

test("the account's words are the person's, once it has those this browser took before it had an account", async ({ page }) => {
  const theirs = ["quik"];
  // Words from before any account answered, on the first page only.
  await page.addInitScript(() => localStorage.getItem("letters.words") ?? localStorage.setItem("letters.words", "Zorvex\nquik"));
  const asked = await serve(page, opened(), account({ words: theirs }));
  await expect.poll(() => named(asked)).toEqual([`PUT ${WORD}Zorvex`]);

  await learn(page, "Blorptix");
  await expect.poll(() => named(asked)).toEqual([`PUT ${WORD}Zorvex`, `PUT ${WORD}Blorptix`]);
  await expect.poll(() => words(page)).toBe("quik\nZorvex\nBlorptix");
  expect(theirs).toEqual(["quik", "Zorvex", "Blorptix"]);

  // A word taken out on another device is not brought back by this one.
  theirs.splice(1, 1);
  await opens(page);
  await expect.poll(() => words(page)).toBe("quik\nBlorptix");
  await rested(page);
  expect(named(asked)).toHaveLength(2);
});

test("a word removed in the settings is taken out of the account", async ({ page }) => {
  const theirs = ["quik", "Zorvex"];
  const asked = await serve(page, opened(), account({ words: theirs }));
  await page.keyboard.press("ControlOrMeta+,");
  await click(page, "Remove quik");
  await expect.poll(() => named(asked)).toEqual([`DELETE ${WORD}quik`]);
  await expect.poll(() => words(page)).toBe("Zorvex");
  expect(theirs).toEqual(["Zorvex"]);
});

const unkept = [
  { name: "will not have is told in the server's words, and not offered again", refusal: { status: 400, body: { code: "invalid", message: "Your dictionary already holds 10000 words." } }, again: 0 },
  { name: "could not be asked for is offered when the page next opens", refusal: { status: 503, body: { code: "unavailable", message: "The server is busy." } }, again: 1 },
];
for (const { name, refusal, again } of unkept) {
  test(`a word the account ${name}`, async ({ page }) => {
    const asked = await serve(page, opened(), account({ refusal }));
    const said = page.waitForEvent("dialog");
    await learn(page, "Zorvex");
    const alert = await said;
    expect(alert.message()).toBe(refusal.body.message);
    await alert.accept();
    // The word holds on this page all the same.
    expect(await words(page)).toBe("Zorvex");

    page.on("dialog", (alert) => alert.accept());
    await opens(page);
    await expect.poll(() => words(page)).toBe(again ? "Zorvex" : "");
    await rested(page);
    await expect.poll(() => named(asked)).toHaveLength(1 + again);
  });
}

test("a host the person always loads pictures from is kept by their account, until they stop trusting it", async ({ page }) => {
  const theirs = [];
  const answer = fetches({ fetch: (_, asked) => asked && picture(INK).bytes });
  const asked = await serve(page, opened(), (method, path, body) => account({ sources: theirs })(method, path) ?? answer(method, path, body), linked);
  await page.mouse.click(600, 300);
  await run(page, "Always load images from this host");
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);
  expect(named(asked)).toEqual([`PUT ${SOURCE}https://pictures.example/`]);

  // The settings list it, as the account wrote it.
  await page.keyboard.press("ControlOrMeta+,");
  const sources = page.getByRole("list", { name: "Trusted sources" });
  await expect(sources.getByRole("listitem", { name: "https://pictures.example/" })).toBeAttached();
  await click(page, "Stop trusting https://pictures.example/");
  await expect.poll(() => named(asked).at(-1)).toBe(`DELETE ${SOURCE}https://pictures.example/`);
  await expect(sources.getByRole("listitem")).toHaveCount(0);
  expect(theirs).toEqual([]);
});

test("a source typed in the settings brings in the pictures on the page that are under it", async ({ page }) => {
  const answer = fetches({ fetch: (_, asked) => asked && picture(INK).bytes });
  const theirs = account();
  const asked = await serve(page, opened(), (method, path, body) => theirs(method, path) ?? answer(method, path, body), linked);
  await page.keyboard.press("ControlOrMeta+,");
  await click(page, "Trust a source", "textbox");
  await page.keyboard.type("pictures.example");
  await page.keyboard.press("Enter");
  await expect.poll(() => told(asked).filter((line) => line.startsWith("POST /fetch/picture"))).toEqual([asking(false), asking(true)]);
  await page.keyboard.press("Escape");
  await expect.poll(() => inked(page)).toBeGreaterThan(1000);
});

const typed = [
  { name: "is trusted as the account writes it", refusal: undefined, listed: 1 },
  { name: "that is none is refused in the server's words", refusal: { status: 400, body: { code: "invalid", message: "That is not a source of pictures." } }, listed: 0 },
];
for (const { name, refusal, listed } of typed) {
  test(`a source typed in the settings ${name}`, async ({ page }) => {
    const asked = await serve(page, opened(), account({ refusal }));
    await page.keyboard.press("ControlOrMeta+,");
    await click(page, "Trust a source", "textbox");
    await page.keyboard.type("img.example.org");
    await page.keyboard.press("Enter");
    await expect.poll(() => named(asked)).toEqual([`PUT ${SOURCE}img.example.org`]);
    const dialog = page.getByRole("dialog", { name: "Settings" });
    await expect(dialog.getByRole("listitem", { name: "https://img.example.org/" })).toHaveCount(listed);
    await expect(dialog.getByRole("alert", { name: "That is not a source of pictures." })).toHaveCount(1 - listed);
  });
}

test("text in a language the server has a dictionary for is checked against it, once the document names the language", async ({ page }) => {
  const german = { aff: "SET UTF-8\nTRY esn\n", dic: "2\nStraße\nÜbung\n" };
  const frames = [];
  const held = async (method, path) => {
    if (path === "/api/v1/dictionaries") return { body: ["de-DE", "fr"] };
    const [, file] = path.match(/^\/api\/v1\/dictionaries\/de-DE\/(aff|dic)$/) ?? [];
    if (file) frames.push(await page.evaluate(() => window.letters?.frames ?? 0));
    return file && { body: german[file], type: "text/plain; charset=utf-8" };
  };
  const written = { format: 1, language: "de", blocks: [{ id: "p", type: "paragraph", content: [{ text: "" }] }] };
  const asked = await serve(page, opened(), held, written);
  // Only the language that is written in is fetched, by the name the server has it under.
  await expect.poll(() => told(asked).sort()).toEqual(["GET /api/v1/dictionaries/de-DE/aff", "GET /api/v1/dictionaries/de-DE/dic"]);
  // The first frame did not wait for it.
  expect(Math.min(...frames)).toBeGreaterThan(0);
  await rested(page);
  // A dictionary judges only words in its own letters, so the wrong word is spelled with them.
  await page.keyboard.type("Strase ");
  await rested(page);
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  await page.locator("textarea").dispatchEvent("contextmenu");
  // The word is marked, and the correction is the German dictionary's.
  await expect(page.getByRole("menuitem", { name: "Add to my dictionary" })).toBeAttached();
  await expect(page.getByRole("menuitem", { name: "Straße" })).toBeAttached();
});

/** The served document in a folder of a space, and what the server says of a document there. */
const HOUSE = { space: "family", path: "House/Minutes.let" };
const listed = (name, id = opened().id) => ({ kind: "document", name, space: "family", path: `House/${name}.let`, id, modified: 0, role: "editor" });

const renames = [
  { name: "takes", answer: { body: listed("Agenda") }, title: "Agenda" },
  { name: "refuses", answer: { status: 409, body: { code: "name_taken", message: "Another document here is called Agenda." } }, title: "Minutes" },
];
for (const { name, answer, title } of renames) {
  test(`the document is renamed by its server, which ${name} the name`, async ({ page }) => {
    const asked = await serve(page, opened(HOUSE), (method, path) => (path === `${DOCUMENT}/rename` ? answer : undefined));
    await run(page, "Rename or move");
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.type("Agenda");
    await page.keyboard.press("Enter");
    await expect.poll(() => told(asked)).toEqual(['POST /rename {"name":"Agenda"}']);
    await expect(page).toHaveTitle(title);
    // The name field stays with what was typed and says why, or the bar has the new name.
    const said = answer.status ? page.getByRole("alert", { name: "Another document here is called Agenda." }) : page.getByRole("button", { name: "Agenda", exact: true });
    await expect(said).toBeAttached();
  });
}

test("a new document is started beside this one, and the page goes to it once what was written is kept", async ({ page }) => {
  const [host, made] = [room(), listed("Untitled", "0190f3a2-7c1e-7b4a-9c55-000000000002")];
  const documents = "/api/v1/spaces/family/documents";
  const asked = await serve(page, opened(HOUSE), (method, path) => (path === documents ? { status: 201, body: made } : undefined), undefined, host);
  await page.route(`**/d/${made.id}`, (route) => route.fulfill({ contentType: "text/html", body: "<title>The new one</title>" }));
  await page.keyboard.type("Kept first.");
  await run(page, "New document");
  await expect(page).toHaveTitle("The new one");
  expect(told(asked)).toEqual([`POST ${documents} {"folder":"House"}`]);
  expect((await host.texts()).join()).toContain("Kept first.");
  host.close();
});

test("the person's other documents are on the server's page for the folder", async ({ page }) => {
  await serve(page, opened(HOUSE));
  await page.route("**/s/family/House", (route) => route.fulfill({ contentType: "text/html", body: "<title>House</title>" }));
  await run(page, "Open");
  await expect(page).toHaveTitle("House");
});

test("a document started without a name takes its first heading's, on the server too", async ({ page }) => {
  const named = (method, path) => (path === `${DOCUMENT}/rename` ? { body: listed("Letters in the browser") } : undefined);
  const asked = await serve(page, opened({ ...HOUSE, name: "Untitled", named: false }), named);
  // The caret leaves the heading.
  await page.keyboard.press("ControlOrMeta+Home");
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => told(asked)).toEqual(['POST /rename {"name":"Letters in the browser"}']);
  await expect(page).toHaveTitle("Letters in the browser");
});

const links = [
  ["allows", true],
  ["withholds", false],
];
for (const [name, allowed] of links) {
  test(`what a share link ${name} of the document is ${allowed ? "" : "not "}offered, by the browser's own print too`, async ({ page }) => {
    await serve(page, opened({ allows: { download: allowed, copy: allowed, print: allowed } }));
    await click(page, "More");
    await expect(page.getByRole("menuitem", { name: "Settings" })).toBeAttached();
    for (const item of ["Save a copy", "Export"]) {
      await expect(page.getByRole("menuitem", { name: item })).toHaveCount(allowed ? 1 : 0);
    }
    await page.emulateMedia({ media: "print" });
    await expect(page.locator("canvas")).toBeVisible({ visible: allowed });
  });
}

const grace = { to: { kind: "person", id: "sub-grace", name: "Grace Hopper" }, role: "reader" };
const friends = { to: { kind: "group", name: "friends" }, role: "reader" };
const link = { id: "l1", label: "Accountant", role: "reader", created: 1794000000, allows: {} };
const REFUSAL = { status: 403, body: { code: "forbidden", message: "Only the owner says who a document is shared with." } };
/** A server that shares the document with Grace and answers what the Share dialog asks, or `refuses` to change anything. */
const sharing = (refuses) => (method, path, body) => {
  if (path === "/api/v1/server") return { body: { product: "letters", name: "Letters", version: "0.1.0", protocols: [1], links: true } };
  if (method !== "GET" && refuses) return REFUSAL;
  if (path === `${DOCUMENT}/shares`) return { body: { people: method === "PUT" ? JSON.parse(body).people : [grace], links: [] } };
  if (path === `${DOCUMENT}/links`) return { status: 201, body: { link, url: "https://letters.example/l/abc" } };
  if (path === `${DOCUMENT}/access`) return { body: [{ time: 1794000000, who: grace.to, action: "open", role: "reader", address: "192.0.2.7", browser: "Firefox" }] };
  return path === `${DOCUMENT}/links/l1` ? { status: 204 } : undefined;
};
const shared = (page, name) => page.getByRole("list", { name: "Shared with" }).getByRole("listitem", { name });
/** Adds the group `friends` as readers in the Share dialog. */
const add = async (page) => {
  await click(page, "Share");
  await expect(shared(page, /Grace Hopper/)).toBeAttached();
  await click(page, "Add a group", "textbox");
  await page.keyboard.type("friends");
  await click(page, "Add reader");
};

test("the owner shares the document through its server: with a group, and by a link that is then revoked", async ({ page }) => {
  const asked = await serve(page, opened({ role: "owner" }), sharing(false));
  await add(page);
  await expect(shared(page, /friends/)).toBeAttached();
  await click(page, "New link");
  await page.keyboard.type("Accountant");
  await click(page, "Make link");
  await expect(page.getByRole("button", { name: "Copy link" })).toBeAttached();
  await click(page, "Revoke");
  await expect.poll(() => told(asked).at(-1)).toBe("DELETE /links/l1");

  const [about, shares, put, post] = told(asked);
  expect([about, shares]).toEqual(["GET /api/v1/server", "GET /shares"]);
  expect(JSON.parse(put.replace("PUT /shares ", "")).people).toEqual([grace, friends]);
  expect(JSON.parse(post.replace("POST /links ", ""))).toMatchObject({ label: "Accountant", role: "reader" });
  expect(await page.evaluate(() => window.blocked)).toEqual([]);
});

test("a change to the shares that the server refuses is told in its words, and the list is the server's again", async ({ page }) => {
  const asked = await serve(page, opened({ role: "owner" }), sharing(true));
  await add(page);
  await expect(page.getByRole("dialog", { name: "Share" }).getByRole("alert", { name: REFUSAL.body.message })).toBeAttached();
  await expect(shared(page, /friends/)).toHaveCount(0);
  await expect(shared(page, /Grace Hopper/)).toBeAttached();
  expect(told(asked).filter((line) => line === "GET /shares")).toHaveLength(2);
});

test("who reached the document through a share is asked of its server when the owner turns to Activity", async ({ page }) => {
  const asked = await serve(page, opened({ role: "owner" }), sharing(false));
  await click(page, "Share");
  await expect(shared(page, /Grace Hopper/)).toBeAttached();
  await click(page, "Activity", "tab");
  await expect(page.getByRole("dialog", { name: "Share" }).getByRole("listitem", { name: /Grace Hopper/ })).toBeAttached();
  expect(told(asked).at(-1)).toBe("GET /access");
});

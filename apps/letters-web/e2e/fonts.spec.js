// The fonts a writer chose beyond the bundled ones: a family of the Google Fonts library is fetched through
// the server for whoever writes, and kept in the document, where a reader finds it.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { entry, file } from "./page.js";
import { opened, serve } from "./server.js";

// A font file that stands for whatever the library serves.
const FACE = readFileSync(new URL("../../../crates/letters-fonts/fonts/FiraCode.ttf", import.meta.url));
const NAME = `assets/${createHash("sha256").update(FACE).digest("hex")}.ttf`;
// The library as the server lists it: one family, of one face.
const LIBRARY = "# families\nLobster\t400\t0\n";
const LISTED = [{ family: "Lobster", weight: 400, italic: false, asset: NAME }];
const DOCUMENT = (said) => `/api/v1/documents/${said.id}`;

/** A document whose body is set in a family of the library, which its style lists the files of if `fonts` is given. */
const styled = (over, fonts) => opened({ style: { format: 1, types: { body: { family: "Lobster" } }, ...(fonts && { fonts }) }, ...over });

/** A server that has the library, and the document's font file if `held`. */
const library = (said, held) => (method, path) => {
  if (method === "GET" && path === "/api/v1/fonts") return { body: LIBRARY, type: "text/plain" };
  if (method === "GET" && path.startsWith(`${DOCUMENT(said)}/fonts/`)) return { body: FACE, type: "font/ttf" };
  if (method === "GET" && held && path === `${DOCUMENT(said)}/${NAME}`) return { body: FACE, type: "application/octet-stream" };
  return method === "PUT" ? { status: 204 } : undefined;
};

const faces = (asked) => asked.filter((request) => request.path.includes("/fonts/"));

test("a writer's family of the library is fetched once through the server and kept in the document", async ({ page }) => {
  const said = styled();
  const asked = await serve(page, said, library(said, false));
  await expect.poll(() => asked.filter((request) => request.method === "PUT").map((request) => request.path)).toEqual([`${DOCUMENT(said)}/${NAME}`]);
  expect(faces(asked).map((request) => request.path)).toEqual([`${DOCUMENT(said)}/fonts/Lobster/400/upright`]);

  // The document's file holds the face, and its style says which family it is for.
  const kept = page.waitForEvent("download");
  await page.keyboard.press("ControlOrMeta+s");
  const zip = await readFile(await (await kept).path());
  await expect.poll(() => JSON.parse(entry(zip, "style.json")).fonts).toEqual(LISTED);
  expect(file(zip, NAME)?.equals(FACE)).toBe(true);
  expect(await page.evaluate(() => window.blocked)).toEqual([]);
});

test("a reader is given the font file the document holds, and the library is asked for nothing", async ({ page }) => {
  const said = styled({ role: "reader" }, LISTED);
  const asked = await serve(page, said, library(said, true));
  await expect.poll(() => asked.map((request) => request.path)).toContain(`${DOCUMENT(said)}/${NAME}`);
  expect(faces(asked)).toEqual([]);
  expect(asked.filter((request) => request.path === "/api/v1/fonts")).toEqual([]);
});

test("a document that is not trusted has no family fetched for it", async ({ page }) => {
  const said = styled({ trust: "safe" });
  const asked = await serve(page, said, library(said, false));
  await page.keyboard.type("Written here.");
  await expect.poll(() => asked.some((request) => request.path === "/api/v1/fonts")).toBe(true);
  expect(faces(asked)).toEqual([]);
});

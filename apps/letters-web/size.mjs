// Reports what the browser downloads and which crates the code in it comes from.
//
//   node size.mjs <dist> <named.wasm>
//
// The files in `dist` are measured as they are and compressed. The named file is the wasm with its function
// names left in, which is what says whose each function is.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { brotliCompressSync, gzipSync, constants } from "node:zlib";

const [dist, namedPath] = process.argv.slice(2);
const read = (...path) => readFileSync(join(dist, ...path));
const served = read("letters_bg.wasm");
const named = readFileSync(namedPath);

/** The sections of a module, each as `{ id, name, bytes }`. */
function sections(wasm) {
  const found = [];
  let at = 8;
  while (at < wasm.length) {
    const id = wasm[at];
    const [size, start] = leb(wasm, at + 1);
    const bytes = wasm.subarray(start, start + size);
    found.push({ id, name: id === 0 ? string(bytes, 0)[0] : "", bytes });
    at = start + size;
  }
  return found;
}

/** An unsigned LEB128 number at `at`, and where the bytes after it start. */
function leb(bytes, at) {
  let [value, shift] = [0, 0];
  for (;;) {
    const byte = bytes[at++];
    value += (byte & 0x7f) * 2 ** shift;
    if (byte < 0x80) return [value, at];
    shift += 7;
  }
}

function string(bytes, at) {
  const [length, start] = leb(bytes, at);
  return [bytes.subarray(start, start + length).toString("utf8"), start + length];
}

/** How many functions a module imports, which are numbered before its own. */
function importedFunctions(bytes) {
  let [count, at] = leb(bytes, 0);
  let functions = 0;
  for (; count > 0; count--) {
    at = string(bytes, string(bytes, at)[1])[1];
    const kind = bytes[at++];
    if (kind === 0) functions++;
    // A function names its type. A table, a memory and a global describe themselves.
    if (kind === 0) at = leb(bytes, at)[1];
    else if (kind === 1) at = limits(bytes, at + 1);
    else if (kind === 2) at = limits(bytes, at);
    else at += 2;
  }
  return functions;
}

function limits(bytes, at) {
  const flags = bytes[at];
  at = leb(bytes, at + 1)[1];
  return flags & 1 ? leb(bytes, at)[1] : at;
}

/** The names of a module's functions, by index. */
function functionNames(bytes) {
  const names = new Map();
  let at = string(bytes, 0)[1];
  while (at < bytes.length) {
    const kind = bytes[at];
    const [size, start] = leb(bytes, at + 1);
    if (kind === 1) {
      let [count, next] = leb(bytes, start);
      for (; count > 0; count--) {
        const [index, afterIndex] = leb(bytes, next);
        const [name, afterName] = string(bytes, afterIndex);
        names.set(index, name);
        next = afterName;
      }
    }
    at = start + size;
  }
  return names;
}

/**
 * The crate a function belongs to, read from its mangled name: the crate that defines it, or for a method the
 * crate whose `impl` it is in. Names that are not Rust's come from the compiler's own support code.
 */
function crate(name) {
  if (!name.startsWith("_R")) return "(not Rust)";
  const disambiguator = /^s[0-9A-Za-z]*_/;
  let rest = name.slice(2);
  // A path leads with what it is nested in, down to the crate at its root.
  for (;;) {
    if (rest[0] === "N") rest = rest.slice(2);
    else if (rest[0] === "I") rest = rest.slice(1);
    else if (rest[0] === "M" || rest[0] === "X") rest = rest.slice(1).replace(disambiguator, "");
    else break;
  }
  const root = rest.match(/^C(?:s[0-9A-Za-z]*_)?(\d+)_?/);
  return root ? rest.slice(root[0].length, root[0].length + Number(root[1])) : "(generic)";
}

/** The size of the code in a module, by crate. */
function codeByCrate(wasm) {
  const all = sections(wasm);
  const code = all.find((section) => section.id === 10).bytes;
  const imports = all.find((section) => section.id === 2);
  const names = functionNames(all.find((section) => section.name === "name").bytes);
  const sizes = new Map();
  let [count, at] = leb(code, 0);
  for (let index = imports ? importedFunctions(imports.bytes) : 0; count > 0; count--, index++) {
    const [size, start] = leb(code, at);
    const owner = crate(names.get(index) ?? "");
    sizes.set(owner, (sizes.get(owner) ?? 0) + size + (start - at));
    at = start + size;
  }
  return [...sizes].sort((a, b) => b[1] - a[1]);
}

const kib = (bytes) => `${(bytes / 1024).toFixed(0).padStart(7)} KiB`;
const row = (label, ...sizes) => console.log(`  ${label.padEnd(22)}${sizes.map(kib).join("")}`);
const size = (id) => sections(served).find((section) => section.id === id)?.bytes.length ?? 0;
const brotli = (bytes) => brotliCompressSync(bytes, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length;
/** A download as it is, gzipped and brotlied. Files that travel apart are compressed apart. */
const download = (files) => [(bytes) => bytes.length, (bytes) => gzipSync(bytes, { level: 9 }).length, brotli].map(
  (measure) => files.reduce((sum, bytes) => sum + measure(bytes), 0),
);

/** The files of a folder that the editor fetches. Their licences travel with them and are never asked for. */
const names = (name) => readdirSync(join(dist, name)).filter((file) => !file.endsWith(".txt"));
const folder = (name) => names(name).map((file) => read(name, file));
const [maths, dictionaries] = ["maths", "dictionaries"].map(folder);
/** The first frame of a document in Latin script waits for this stand-in for the first font, and no other. */
const FIRST = "Inter-Latin.ttf";
const fonts = names("fonts").filter((file) => file !== FIRST).map((file) => read("fonts", file));
/** The faces that back up the fonts. A document fetches one when it first shows a character of its script. */
const fallback = readdirSync(join(dist, "fallback")).filter((file) => !file.endsWith(".txt"));
/** The scripts `letters.js` imports, which `wasm-bindgen` writes beside it. */
const snippets = readdirSync(join(dist, "snippets"), { recursive: true })
  .filter((file) => file.endsWith(".js"))
  .map((file) => read("snippets", file));
const downloads = [
  ["letters_bg.wasm", [served]],
  ["letters.js", [read("letters.js"), ...snippets]],
  [`fonts/${FIRST}`, [read("fonts", FIRST)]],
];
// A document with maths has the maths faces before its first frame, too.
const later = [
  [`fonts (${fonts.length} files)`, fonts],
  [`maths (${maths.length} files)`, maths],
  [`dictionaries (${dictionaries.length})`, dictionaries],
  ["exports, when asked for", [read("export_bg.wasm"), read("export.js")]],
];
console.log(`${"What the browser downloads".padEnd(24)}${["as served", "gzip", "brotli"].map((head) => head.padStart(11)).join("")}`);
downloads.forEach(([label, files]) => row(label, ...download(files)));
row("for the first frame", ...download(downloads.flatMap(([, files]) => files)));
later.forEach(([label, files]) => row(label, ...download(files)));
readdirSync(join(dist, "words")).forEach((file) => row(`words/${file}`.slice(0, 22), ...download([read("words", file)])));
fallback.forEach((file) => row(`fallback/${file}`.slice(0, 22), ...download([read("fallback", file)])));
console.log("What the wasm holds");
row("code", size(10));
row("data", size(11));
row("the rest", served.length - size(10) - size(11));
console.log("Its code by crate");
const crates = codeByCrate(named);
const shown = crates.filter(([, bytes]) => bytes >= 16 * 1024);
shown.forEach(([name, bytes]) => row(name, bytes));
row(`${crates.length - shown.length} smaller crates`, crates.slice(shown.length).reduce((sum, [, bytes]) => sum + bytes, 0));

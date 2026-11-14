// Reports what the browser downloads and which crates the code in it comes from.
//
//   node size.mjs <served.wasm> <named.wasm>
//
// The served file is measured as it is and compressed. The named file is the same module with its function
// names left in, which is what says whose each function is.

import { readFileSync, readdirSync } from "node:fs";
import { brotliCompressSync, gzipSync, constants } from "node:zlib";

const [served, named] = process.argv.slice(2).map((path) => readFileSync(path));
const FONTS = new URL("../../crates/letters-fonts/fonts/", import.meta.url);

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

const kib = (bytes) => `${(bytes / 1024).toFixed(0).padStart(6)} KiB`;
const row = (label, bytes, note = "") => console.log(`  ${label.padEnd(24)}${kib(bytes)}${note && `  ${note}`}`);
const size = (id) => sections(served).find((section) => section.id === id)?.bytes.length ?? 0;
const fonts = readdirSync(FONTS).filter((file) => file.endsWith(".ttf"));
const fontBytes = fonts.reduce((sum, file) => sum + readFileSync(new URL(file, FONTS)).length, 0);

console.log("The wasm the browser downloads");
row("as served", served.length);
row("gzip", gzipSync(served, { level: 9 }).length);
row("brotli", brotliCompressSync(served, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length);
console.log("What it holds");
row("code", size(10));
row("data", size(11), `of which ${fonts.length} bundled fonts ${kib(fontBytes).trim()}`);
row("the rest", served.length - size(10) - size(11));
console.log("Code by crate");
const crates = codeByCrate(named);
const shown = crates.filter(([, bytes]) => bytes >= 16 * 1024);
shown.forEach(([name, bytes]) => row(name, bytes));
row(`${crates.length - shown.length} smaller crates`, crates.slice(shown.length).reduce((sum, [, bytes]) => sum + bytes, 0));

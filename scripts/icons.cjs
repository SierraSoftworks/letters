#!/usr/bin/env node
// Draws everything under `assets/icons/` that is not an SVG from the SVGs there, with Playwright's Chromium.
// The 16 and 32 pixel drawings are their own, set on whole pixels: the rest is the full drawing.
// `web/` is what a browser asks a site for. `bleed.svg` is the app icon for a system that cuts the tile itself.
//
//   node scripts/icons.cjs
const { chromium } = require("playwright");
const { readFileSync, writeFileSync, mkdirSync } = require("node:fs");
const { join } = require("node:path");

const ICONS = join(__dirname, "..", "assets", "icons");
const SIZES = [16, 32, 48, 64, 128, 256, 512, 1024];
const drawing = (name, size) => readFileSync(join(ICONS, `${name}${size <= 16 ? "-16" : size <= 32 ? "-32" : ""}.svg`), "utf8");

// An ICNS and an ICO are both a short table in front of PNGs.
const ICNS = { icp4: 16, icp5: 32, icp6: 64, ic07: 128, ic08: 256, ic09: 512, ic10: 1024, ic11: 32, ic12: 64, ic13: 256, ic14: 512 };
function icns(pngs) {
  const parts = Object.entries(ICNS).map(([type, size]) => {
    const head = Buffer.alloc(8);
    head.write(type, "ascii");
    head.writeUInt32BE(pngs[size].length + 8, 4);
    return Buffer.concat([head, pngs[size]]);
  });
  const head = Buffer.alloc(8);
  head.write("icns", "ascii");
  head.writeUInt32BE(8 + parts.reduce((sum, part) => sum + part.length, 0), 4);
  return Buffer.concat([head, ...parts]);
}
function ico(pngs, sizes) {
  const head = Buffer.alloc(6 + 16 * sizes.length);
  head.writeUInt16LE(1, 2);
  head.writeUInt16LE(sizes.length, 4);
  let at = head.length;
  sizes.forEach((size, i) => {
    const entry = 6 + 16 * i;
    // A byte each, where 0 stands for 256.
    head.writeUInt8(size & 255, entry);
    head.writeUInt8(size & 255, entry + 1);
    head.writeUInt16LE(1, entry + 4);
    head.writeUInt16LE(32, entry + 6);
    head.writeUInt32LE(pngs[size].length, entry + 8);
    head.writeUInt32LE(at, entry + 12);
    at += pngs[size].length;
  });
  return Buffer.concat([head, ...sizes.map((size) => pngs[size])]);
}

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ deviceScaleFactor: 1 });
  const draw = async (svg, size) => {
    await page.setViewportSize({ width: size, height: size });
    await page.setContent(`<style>html,body{margin:0;background:transparent}svg{display:block;width:${size}px;height:${size}px}</style>${svg}`);
    return page.screenshot({ omitBackground: true });
  };
  mkdirSync(join(ICONS, "png"), { recursive: true });
  const sets = {};
  for (const name of ["app", "file"]) {
    sets[name] = {};
    for (const size of SIZES) {
      sets[name][size] = await draw(drawing(name, size), size);
      if (size <= 512) writeFileSync(join(ICONS, "png", `${name}-${size}.png`), sets[name][size]);
    }
  }
  // A page's icons: the tab's is `favicon.ico`, and the bar of the server's pages shows the 32 pixel drawing.
  const web = { "apple-touch-icon": ["bleed", 180], "icon-192": ["app", 192], "icon-512": ["app", 512], "icon-maskable-512": ["bleed", 512] };
  mkdirSync(join(ICONS, "web"), { recursive: true });
  for (const [file, [name, size]] of Object.entries(web)) {
    writeFileSync(join(ICONS, "web", `${file}.png`), await draw(readFileSync(join(ICONS, `${name}.svg`), "utf8"), size));
  }
  await browser.close();
  writeFileSync(join(ICONS, "letters.icns"), icns(sets.app));
  writeFileSync(join(ICONS, "document.icns"), icns(sets.file));
  writeFileSync(join(ICONS, "favicon.ico"), ico(sets.app, [16, 32, 48]));
  // What Windows shows for the program's file, at every size it asks for.
  writeFileSync(join(ICONS, "letters.ico"), ico(sets.app, [16, 32, 48, 64, 128, 256]));
})();

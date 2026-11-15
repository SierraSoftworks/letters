// Builds what the tests run with cargo, before any test does: the `page` and `room` examples behind
// `server.js`, and with `LETTERS_WEB_DIST` the server of `server.spec.js`. Left to the first test that
// runs one, a build is on that test's time, and with `LETTERS_WEB_DIST` set the examples take minutes:
// letters-server is built again with the editor in it.

import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

export default () => {
  const cwd = fileURLToPath(new URL("..", import.meta.url));
  const build = (...what) => execFileSync("cargo", ["build", "--quiet", "-p", "letters-web", ...what], { cwd, stdio: "inherit" });
  build("--example", "page", "--example", "room");
  if (process.env.LETTERS_WEB_DIST) build("--release", "--example", "server");
};

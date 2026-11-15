import { defineConfig, devices } from "@playwright/test";

const PORT = 8187;

// Run `./build.sh` first: the tests load what it leaves in `dist/`.
export default defineConfig({
  testDir: "e2e",
  globalSetup: "./e2e/built.js",
  forbidOnly: !!process.env.CI,
  use: { baseURL: `http://localhost:${PORT}/` },
  // Firefox is the reference browser, and a failure there blocks a merge. The others must pass the same tests.
  projects: [
    { name: "firefox", use: devices["Desktop Firefox"] },
    { name: "chromium", use: devices["Desktop Chrome"] },
  ],
  webServer: { command: `node serve.mjs ${PORT}`, url: `http://localhost:${PORT}/`, reuseExistingServer: !process.env.CI },
});

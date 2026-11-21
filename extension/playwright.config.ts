import { defineConfig } from "@playwright/test";

// The extension is loaded into a real Chromium (Chrome itself has stopped allowing unpacked extensions to
// be loaded from the command line, and Chromium is what Playwright drives). The camera and microphone
// are the browser's built-in fake devices, which make a moving test picture and a tone, so a recording
// has something real to hold. The tests build first: `npm run e2e`.
export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.spec.ts",
  workers: 1,
  fullyParallel: false,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  timeout: 60_000,
  expect: { timeout: 10_000 },
});

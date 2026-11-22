import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "@playwright/test";

import { developmentExtensionId } from "./scripts/extension-id.mjs";

// The extension is loaded into a real Chromium (Chrome itself has stopped allowing unpacked extensions to
// be loaded from the command line, and Chromium is what Playwright drives). The camera and microphone
// are the browser's built-in fake devices, which make a moving test picture and a tone, so a recording
// has something real to hold. The tests build first: `npm run e2e`.
//
// The upload tests talk to the real API and the Postgres, Redis and object storage behind it
// (`docker compose -f deploy/docker-compose.yml up -d`). The API runs with its log going to a file, because the
// development mail transport prints each message there: that is where a test reads the link a person would have
// clicked. It trusts this extension's id, and gets its own Redis database, emptied before the run, so the
// limits of earlier runs do not count against this one.

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const logs = path.join(here, "e2e", ".logs");
fs.mkdirSync(logs, { recursive: true });

export const API_LOG = path.join(logs, "api.log");
export const E2E_REDIS_URL = process.env.E2E_REDIS_URL ?? "redis://127.0.0.1:6379/14";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.spec.ts",
  globalSetup: "./e2e/global-setup.ts",
  workers: 1,
  fullyParallel: false,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  webServer: {
    // Compiles the first time, which can take minutes.
    command: `cargo run -q -p zeno-api --bin api > "${API_LOG}" 2>&1`,
    cwd: root,
    url: "http://127.0.0.1:8080/ready",
    timeout: 20 * 60_000,
    // The log above is how tests read email, so an API somebody else started cannot be reused.
    reuseExistingServer: false,
    env: {
      LOG_FORMAT: "json",
      REDIS_URL: E2E_REDIS_URL,
      BIND_ADDR: "127.0.0.1:8080",
      CLIENT_URL: "http://localhost:3000",
      EXTENSION_ORIGINS: `chrome-extension://${developmentExtensionId()}`,
    },
  },
});

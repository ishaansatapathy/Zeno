import fs from "node:fs";
import path from "node:path";

import { defineConfig, devices } from "@playwright/test";

// End-to-end tests drive a real browser against the real thing: this web app, the Rust API, and
// the Postgres, Redis and object storage it needs (`docker compose -f deploy/docker-compose.yml up -d`).
//
//   npm run e2e
//
// The API runs with its log going to a file, in JSON, because the development mail transport
// prints each message there: that is where the tests read the link a person would have clicked.
// It gets its own Redis database, emptied before the run, so the sign-up and sign-in limits of
// earlier runs do not count against this one.

const root = path.resolve(__dirname, "..");
const logs = path.join(__dirname, "e2e", ".logs");
fs.mkdirSync(logs, { recursive: true });

export const API_LOG = path.join(logs, "api.log");
export const E2E_REDIS_URL = process.env.E2E_REDIS_URL ?? "redis://127.0.0.1:6379/15";

const ci = !!process.env.CI;

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.spec.ts",
  globalSetup: "./e2e/global-setup.ts",
  // One server, one database: tests share them, so they take turns.
  workers: 1,
  fullyParallel: false,
  retries: ci ? 1 : 0,
  reporter: ci ? [["list"], ["html", { open: "never" }]] : "list",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  use: {
    baseURL: "http://localhost:3000",
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: [
    {
      // Compiles the first time, which can take minutes.
      command: `cargo run -q -p zeno-api --bin api > "${API_LOG}" 2>&1`,
      cwd: root,
      url: "http://127.0.0.1:8080/ready",
      timeout: 20 * 60_000,
      // The log above is how tests read email, so an API somebody else started cannot be reused.
      reuseExistingServer: false,
      env: { LOG_FORMAT: "json", REDIS_URL: E2E_REDIS_URL, BIND_ADDR: "127.0.0.1:8080", CLIENT_URL: "http://localhost:3000" },
    },
    {
      command: ci ? "npm run start" : "npm run dev",
      url: "http://localhost:3000/login",
      timeout: 120_000,
      reuseExistingServer: !ci,
      env: { API_URL: "http://127.0.0.1:8080" },
    },
  ],
});

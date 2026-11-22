// The real API, for the upload tests: making an account, signing a browser in, and asking what Zeno holds.

import fs from "node:fs";

import { expect, type BrowserContext } from "@playwright/test";

import { API_LOG } from "../playwright.config";

/** `localhost`, not 127.0.0.1: the extension is built to reach `localhost`, and a cookie belongs to the name it was set for. */
export const API = "http://localhost:8080";
export const PASSWORD = "correct horse battery staple";

let counter = 0;
const unique = (name: string) => `${name}.${Date.now().toString(36)}${counter++}@example.com`;

/** The newest verify-email link the API has "emailed" to `email`, read from its log. */
async function verificationToken(email: string): Promise<string> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    let found: string | undefined;
    for (const line of fs.readFileSync(API_LOG, "utf8").split("\n")) {
      if (!line.startsWith("{")) continue;
      try {
        const entry = JSON.parse(line) as { fields?: { message?: string; to?: string } };
        if (entry.fields?.to !== email) continue;
        const match = /\/verify-email\/([A-Za-z0-9_-]+)/.exec(entry.fields.message ?? "");
        if (match) found = match[1];
      } catch {
        /* not a log line we care about */
      }
    }
    if (found) return found;
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`No verification email for ${email} in the API log`);
}

/**
 * A confirmed account, made through the API, and the browser signed in as it: the session cookies go into the
 * browser's own jar, which is what a person signing in on the web would have done.
 */
export async function signedInAs(context: BrowserContext, role: "creator" | "student" = "creator"): Promise<{ email: string }> {
  const email = unique("maker");
  const registered = await context.request.post(`${API}/v1/auth/register`, { data: { name: "Asha Rao", email, password: PASSWORD, role } });
  expect(registered.status()).toBe(202);
  const verified = await context.request.post(`${API}/v1/auth/verify-email`, { data: { token: await verificationToken(email) } });
  expect(verified.status()).toBe(200);
  const login = await context.request.post(`${API}/v1/auth/login`, { data: { email, password: PASSWORD } });
  expect(login.status()).toBe(200);
  return { email };
}

/** Asks the API something, as the signed-in browser. */
export async function ask<T = unknown>(context: BrowserContext, path: string): Promise<{ status: number; body: T }> {
  const reply = await context.request.get(`${API}${path}`);
  return { status: reply.status(), body: (await reply.json().catch(() => null)) as T };
}

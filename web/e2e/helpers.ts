import { expect, type APIRequestContext, type Page, type PlaywrightWorkerArgs } from "@playwright/test";

import { mailedLink } from "./mail";

export const PASSWORD = "correct horse battery staple";

let counter = 0;
/** An address no earlier run has used, so tests never find each other's accounts. */
export const unique = (name: string) => `${name}.${Date.now().toString(36)}${counter++}@example.com`;

/** An account that is already confirmed, made through the API: for tests about what comes after. */
export async function confirmedAccount(request: APIRequestContext, options: { name?: string; role?: "creator" | "student" } = {}) {
  const email = unique("asha");
  const registered = await request.post("/v1/auth/register", {
    data: { name: options.name ?? "Asha Rao", email, password: PASSWORD, role: options.role ?? "creator" },
  });
  expect(registered.status()).toBe(202);
  const link = await mailedLink(email, "verify-email");
  const verified = await request.post("/v1/auth/verify-email", { data: { token: link.split("/").pop() } });
  expect(verified.status()).toBe(200);
  return email;
}

const shared = new Map<string, Promise<string>>();

/**
 * One confirmed account for every test that only needs somebody signed in. Signing up is limited
 * to ten a hour from one address, and a suite that made an account per test would spend it all, so
 * tests that change the account (a password reset, say) make their own and the rest share this.
 */
export function sharedAccount(playwright: PlaywrightWorkerArgs["playwright"], role: "creator" | "student" = "creator"): Promise<string> {
  let account = shared.get(role);
  if (!account) {
    account = (async () => {
      const request = await playwright.request.newContext({ baseURL: "http://localhost:3000" });
      try {
        return await confirmedAccount(request, { role });
      } finally {
        await request.dispose();
      }
    })();
    shared.set(role, account);
  }
  return account;
}

/** The app's own alerts. Next adds one more (`#__next-route-announcer__`) that reads out page changes. */
export const alertOf = (page: Page) => page.locator('[role="alert"]:not(#__next-route-announcer__)');

/** Fills in and sends the sign-in form. */
export async function signIn(page: Page, email: string, password = PASSWORD) {
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
}

/** Signs the page's browser in without the form, for tests that are about something else. */
export async function signInQuietly(page: Page, email: string) {
  const response = await page.request.post("/v1/auth/login", { data: { email, password: PASSWORD } });
  expect(response.status()).toBe(200);
}

/** Deletes everything in the signed-in person's library, so a test starts from nothing. */
export async function emptyLibrary(page: Page) {
  for (;;) {
    const listed = await page.request.get("/v1/recordings?limit=50");
    expect(listed.status()).toBe(200);
    const { recordings } = await listed.json();
    if (recordings.length === 0) return;
    for (const recording of recordings) {
      const gone = await page.request.delete(`/v1/recordings/${recording.id}`);
      expect([204, 404]).toContain(gone.status());
    }
  }
}

/** The shape of a recording as the tests need it. */
export interface Seeded {
  id: string;
  version: number;
  title: string;
}

/** Puts a recording in the signed-in person's library, as the extension or studio will. */
export async function seedRecording(page: Page, title: string, source = "extension"): Promise<Seeded> {
  const response = await page.request.post("/v1/recordings", { data: { source, title } });
  expect(response.status(), await response.text()).toBe(201);
  const { recording } = await response.json();
  return { id: recording.id, version: recording.version, title: recording.title };
}

/** Makes a share link for a recording of the signed-in person's. */
export async function shareLink(page: Page, recording: string, body: Record<string, unknown> = {}) {
  const response = await page.request.post(`/v1/recordings/${recording}/links`, { data: body });
  expect(response.status(), await response.text()).toBe(201);
  const { link } = await response.json();
  return { id: link.id as string, code: link.code as string };
}

/** Closes a link for good. */
export async function revokeLink(page: Page, recording: string, link: string) {
  const response = await page.request.delete(`/v1/recordings/${recording}/links/${link}`);
  expect(response.status()).toBe(204);
}

/**
 * A video that has been uploaded and handed to the worker. No worker runs in these tests, so it
 * stays `processing`: exactly the state a share page has to be able to show.
 */
export async function processingVideo(page: Page): Promise<string> {
  const size = 1024 * 1024;
  const created = await page.request.post("/v1/videos", { data: { filename: "Lecture.mp4", size_bytes: size, content_type: "video/mp4" } });
  expect(created.status(), await created.text()).toBe(201);
  const body = await created.json();
  const put = await page.request.put(body.upload.parts[0].url, { data: Buffer.alloc(size, 1) });
  expect(put.ok()).toBe(true);
  const done = await page.request.post(`/v1/videos/${body.video.id}/complete`);
  expect(done.status(), await done.text()).toBe(200);
  return body.video.id as string;
}

/** Gives a recording its video. */
export async function attachVideo(page: Page, recording: string, video: string) {
  const response = await page.request.put(`/v1/recordings/${recording}/video`, { data: { video_id: video } });
  expect(response.status(), await response.text()).toBe(200);
}

import { expect, test, type Browser, type Page } from "@playwright/test";

import { alertOf, confirmedAccount, emptyLibrary, seedRecording, sharedAccount, shareLink, signInQuietly } from "./helpers";

const BASE = "http://localhost:3000";

/** Somebody opening a link, in a browser of their own, with no account unless they sign in. */
async function visitor(browser: Browser) {
  const context = await browser.newContext({ baseURL: BASE });
  return { context, page: await context.newPage() };
}

/**
 * The page of a video that is ready. No worker runs in these tests, so the video itself is not there: the
 * link's answer is the one thing made up here. The comments below it are the real API's.
 */
async function pretendReady(page: Page, code: string, title: string) {
  await page.route(`**/v1/s/${code}`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ share: { code, requires_password: false, title, status: "ready", duration_seconds: 125 } }) }),
  );
}

async function say(page: Page, recording: string, code: string, body: string) {
  const response = await page.request.post(`/v1/s/${code}/comments`, { data: { kind: "comment", body, at_ms: 12_000 } });
  expect(response.status(), await response.text()).toBe(201);
  void recording;
}

test("anyone with the link reads what was said, and a person who is not signed in is offered the sign-in", async ({ page, browser, playwright }) => {
  await signInQuietly(page, await sharedAccount(playwright));
  await emptyLibrary(page);
  const recording = await seedRecording(page, "Week 3");
  const link = await shareLink(page, recording.id);
  await say(page, recording.id, link.code, "The part at the start is the one I mean");

  const { context, page: seen } = await visitor(browser);
  await pretendReady(seen, link.code, "Week 3");
  await seen.goto(`/s/${link.code}`);
  await expect(seen.getByRole("heading", { name: /^Comments/ })).toBeVisible();
  await expect(seen.getByText("The part at the start is the one I mean")).toBeVisible();
  await expect(seen.getByRole("listitem").getByRole("button", { name: "Go to 0:12" })).toBeVisible();
  await expect(seen.getByRole("button", { name: "A comment or reaction at 0:12" })).toBeVisible();

  // No account: the attempt says why, and where to go.
  await seen.getByLabel("Add a comment").fill("Can I join in?");
  await seen.getByRole("button", { name: "Comment", exact: true }).click();
  const prompt = alertOf(seen);
  await expect(prompt).toContainText("to leave a comment");
  await expect(prompt.getByRole("link", { name: "Sign in" })).toHaveAttribute("href", `/login?next=${encodeURIComponent(`/s/${link.code}`)}`);
  // Nothing was posted: the words are still in the box, and not in the conversation.
  await expect(seen.getByRole("listitem").filter({ hasText: "Can I join in?" })).toHaveCount(0);
  await expect(seen.getByLabel("Add a comment")).toHaveValue("Can I join in?");
  await context.close();
});

test("a signed-in person comments and removes their own, and the owner can remove anyone's", async ({ page, browser, playwright }) => {
  await signInQuietly(page, await sharedAccount(playwright));
  await emptyLibrary(page);
  const recording = await seedRecording(page, "Week 3");
  const link = await shareLink(page, recording.id);

  const guest = await playwright.request.newContext({ baseURL: BASE });
  const pupil = await confirmedAccount(guest, { role: "student", name: "Priya Nair" });
  await guest.dispose();

  const { context, page: seen } = await visitor(browser);
  await pretendReady(seen, link.code, "Week 3");
  await signInQuietly(seen, pupil);
  await seen.goto(`/s/${link.code}`);
  await expect(seen.getByText("No comments yet. Be the first.")).toBeVisible();

  await seen.getByLabel("Add a comment").fill("Could you go over this once more?");
  await seen.getByRole("button", { name: "Comment", exact: true }).click();
  const mine = seen.getByRole("listitem").filter({ hasText: "Could you go over this once more?" });
  await expect(mine).toBeVisible();
  await expect(mine).toContainText("Priya Nair");
  await expect(mine.getByRole("button", { name: "Remove your comment" })).toBeVisible();

  // It is there for the next person too, and it is theirs to remove only for its author and the owner.
  await seen.reload();
  await expect(mine).toBeVisible();
  const other = await visitor(browser);
  await pretendReady(other.page, link.code, "Week 3");
  await other.page.goto(`/s/${link.code}`);
  await expect(other.page.getByText("Could you go over this once more?")).toBeVisible();
  await expect(other.page.getByRole("button", { name: /Remove/ })).toHaveCount(0);
  await other.context.close();

  const owners = await visitor(browser);
  await pretendReady(owners.page, link.code, "Week 3");
  await signInQuietly(owners.page, await sharedAccount(playwright));
  await owners.page.goto(`/s/${link.code}`);
  await expect(owners.page.getByRole("button", { name: "Remove the comment by Priya Nair" })).toBeVisible();
  await owners.page.getByRole("button", { name: "Remove the comment by Priya Nair" }).click();
  await expect(owners.page.getByText("Could you go over this once more?")).toHaveCount(0);
  await owners.context.close();

  // And it stays gone.
  await seen.reload();
  await expect(seen.getByText("No comments yet. Be the first.")).toBeVisible();
  await context.close();
});

test("the owner closes the conversation from the library and visitors no longer see it", async ({ page, browser, playwright }) => {
  await signInQuietly(page, await sharedAccount(playwright));
  await emptyLibrary(page);
  const recording = await seedRecording(page, "Week 3");
  const link = await shareLink(page, recording.id);
  await say(page, recording.id, link.code, "Still here when it is back on");

  await page.goto("/library");
  const toggle = page.getByRole("button", { name: "Comments on for Week 3" });
  await expect(toggle).toBeVisible();
  await toggle.click();
  await expect(page.getByRole("button", { name: "Comments off for Week 3" })).toBeVisible();

  const { context, page: seen } = await visitor(browser);
  await pretendReady(seen, link.code, "Week 3");
  await seen.goto(`/s/${link.code}`);
  await expect(seen.getByRole("button", { name: /Play/ })).toBeVisible();
  await expect(seen.getByRole("heading", { name: /^Comments/ })).toHaveCount(0);

  // Back on: what was said is still there.
  await page.getByRole("button", { name: "Comments off for Week 3" }).click();
  await expect(page.getByRole("button", { name: "Comments on for Week 3" })).toBeVisible();
  await seen.reload();
  await expect(seen.getByText("Still here when it is back on")).toBeVisible();
  await context.close();
});

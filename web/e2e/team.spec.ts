import { expect, test, type APIResponse, type Browser, type Page, type PlaywrightWorkerArgs } from "@playwright/test";

import { PASSWORD, alertOf, sharedAccount, signInQuietly, unique } from "./helpers";
import { mailedCount, mailedLink } from "./mail";

type Playwright = PlaywrightWorkerArgs["playwright"];

// Signing up is limited to ten an hour from one address, and the other specs have spent most of it, so these tests
// make no accounts of their own. The owner is the creator the other specs share; the person invited is the shared
// student, given a name of their own so the two can be told apart on the screen. (This spec runs last, so the new
// name is seen by nothing else.)
const ownerEmail = (playwright: Playwright) => sharedAccount(playwright, "creator");
let guest: Promise<string> | null = null;
function guestEmail(playwright: Playwright): Promise<string> {
  return (guest ??= (async () => {
    const email = await sharedAccount(playwright, "student");
    const api = await playwright.request.newContext({ baseURL: "http://localhost:3000" });
    try {
      const signedIn = await api.post("/v1/auth/login", { data: { email, password: PASSWORD } });
      expect(signedIn.status()).toBe(200);
      const named = await api.patch("/v1/auth/me", { data: { name: "Ben Iyer" } });
      expect(named.status()).toBe(200);
    } finally {
      await api.dispose();
    }
    return email;
  })());
}

// An account owns at most five workspaces, and the tests share one: each one ends what it made.
test.afterEach(async ({ page, playwright }) => {
  await signInQuietly(page, await ownerEmail(playwright));
  const listed = await (await page.request.get("/v1/workspaces")).json();
  for (const workspace of listed.workspaces as { id: string; role: string }[]) {
    if (workspace.role === "owner") await page.request.delete(`/v1/workspaces/${workspace.id}`);
  }
});

/** Repeats a call that was told to slow down, once the time it names has passed: the specs before this one spent the account's minute. */
async function patiently(run: () => Promise<APIResponse>): Promise<APIResponse> {
  let res = await run();
  for (let tries = 0; tries < 4 && res.status() === 429; tries++) {
    await new Promise((resolve) => setTimeout(resolve, (Number(res.headers()["retry-after"]) || 5) * 1000));
    res = await run();
  }
  return res;
}

/** A second browser, signed in as somebody else. */
async function another(browser: Browser, email: string): Promise<Page> {
  const context = await browser.newContext({ baseURL: "http://localhost:3000" });
  const page = await context.newPage();
  await signInQuietly(page, email);
  return page;
}

/** The owner, signed in, with a workspace of their own just made (through the page's own API). */
async function workspaceOf(page: Page, playwright: Playwright, name = unique("Maths")) {
  const email = await ownerEmail(playwright);
  await signInQuietly(page, email);
  const made = await page.request.post("/v1/workspaces", { data: { name } });
  expect(made.status(), await made.text()).toBe(201);
  const { workspace } = await made.json();
  return { id: workspace.id as string, name, email };
}

/** Invites `email` through the page and returns the path of the emailed link. */
async function invite(page: Page, workspace: string, email: string, role = "member") {
  const before = mailedCount(email, "invite");
  const sent = await page.request.post(`/v1/workspaces/${workspace}/invites`, { data: { email, role } });
  expect(sent.status(), await sent.text()).toBe(201);
  return mailedLink(email, "invite", before);
}

test("a person starts a workspace and is its owner", async ({ page, playwright }) => {
  const email = await ownerEmail(playwright);
  await signInQuietly(page, email);
  await page.goto("/team");
  await expect(page.getByRole("heading", { level: 1 })).toContainText("workspaces.");

  // Too short: said before anything is sent.
  await page.getByLabel("Name").fill("A");
  await page.getByRole("button", { name: "Start workspace" }).click();
  await expect(page.getByText("Name must be 2 to 80 characters.")).toBeVisible();

  const name = unique("Class 9A");
  await page.getByLabel("Name").fill(name);
  await page.getByRole("button", { name: "Start workspace" }).click();
  await expect(page).toHaveURL(/\/team\/[0-9a-f-]{36}$/);
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(name);
  await expect(page.getByText("You are owner")).toBeVisible();
  await expect(page.getByRole("region", { name: "People · 1" })).toContainText(email);

  await page.goto("/team");
  await expect(page.getByRole("link", { name })).toBeVisible();
  await expect(page.getByRole("listitem").filter({ hasText: name })).toContainText("Owner · 1 person");
});

test("an invitation brings in the account it was sent to, and only that one", async ({ page, browser, playwright }) => {
  const { id, name, email: ownerAddress } = await workspaceOf(page, playwright);
  const guestAddress = await guestEmail(playwright);
  const link = await invite(page, id, guestAddress);

  // The owner sees it waiting.
  await page.goto(`/team/${id}`);
  await expect(page.getByRole("listitem").filter({ hasText: guestAddress })).toContainText("Member · until");

  // Somebody else with the link is told whose it is, and it is not used up.
  const stranger = await another(browser, ownerAddress);
  await stranger.goto(link);
  await stranger.getByRole("button", { name: "Accept invitation" }).click();
  await expect(alertOf(stranger)).toContainText("sent to a different email address");
  await expect(stranger.getByRole("button", { name: /Sign out, then sign in/ })).toBeVisible();
  await stranger.context().close();

  // The right account accepts, lands in the workspace, and is a member.
  const ben = await another(browser, guestAddress);
  await ben.goto(link);
  await expect(ben.getByText(`signed in as ${guestAddress}`)).toBeVisible();
  await ben.getByRole("button", { name: "Accept invitation" }).click();
  await expect(ben).toHaveURL(`/team/${id}`);
  await expect(ben.getByRole("heading", { level: 1 })).toHaveText(name);
  await expect(ben.getByText("You are member")).toBeVisible();
  // A member sees names and their own address, not the others, and none of the controls.
  const people = ben.getByRole("region", { name: "People · 2" });
  await expect(people).toContainText("Asha Rao");
  await expect(people).toContainText(guestAddress);
  await expect(people).not.toContainText(ownerAddress);
  await expect(ben.getByRole("heading", { name: "Invite people" })).toHaveCount(0);
  await expect(ben.getByRole("button", { name: `Delete workspace ${name}` })).toHaveCount(0);
  await expect(ben.getByRole("button", { name: "Leave" })).toBeVisible();

  // It works once.
  await ben.goto(link);
  await ben.getByRole("button", { name: "Accept invitation" }).click();
  await expect(alertOf(ben)).toContainText("not valid any more");
  await ben.context().close();

  // The owner now sees two people, and nothing waiting.
  await page.reload();
  await expect(page.getByRole("region", { name: "People · 2" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Waiting for an answer" })).toHaveCount(0);
});

test("an owner changes what somebody is, and removes them", async ({ page, browser, playwright }) => {
  const { id } = await workspaceOf(page, playwright);
  const guestAddress = await guestEmail(playwright);
  const link = await invite(page, id, guestAddress);
  const ben = await another(browser, guestAddress);
  await ben.goto(link);
  await ben.getByRole("button", { name: "Accept invitation" }).click();
  await expect(ben).toHaveURL(`/team/${id}`);

  // Made an admin: they can now invite, but not choose roles.
  await page.goto(`/team/${id}`);
  await page.getByLabel("Role of Ben Iyer").selectOption("admin");
  await expect(page.getByRole("status").filter({ hasText: "Ben Iyer is now admin." })).toBeVisible();
  await ben.reload();
  await expect(ben.getByRole("heading", { name: "Invite people" })).toBeVisible();
  await expect(ben.getByLabel("Role of Asha Rao")).toHaveCount(0);

  // Removed: gone at once.
  await page.getByRole("button", { name: "Remove Ben Iyer" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Ben Iyer was removed." })).toBeVisible();
  await expect(page.getByRole("region", { name: "People · 1" })).toBeVisible();
  await ben.reload();
  await expect(ben.getByRole("heading", { level: 1 })).toHaveText("Not found.");
  await ben.context().close();
});

test("the only owner cannot leave or step down, and is told why", async ({ page, playwright }) => {
  const { id } = await workspaceOf(page, playwright);
  await page.goto(`/team/${id}`);
  await page.getByRole("button", { name: "Leave" }).click();
  await expect(alertOf(page)).toContainText("You are the only owner");
  await page.getByLabel("Role of Asha Rao").selectOption("member");
  await expect(alertOf(page)).toContainText("needs an owner");
  // Still the owner, and still there.
  await page.reload();
  await expect(page.getByText("You are owner")).toBeVisible();
});

test("a member can leave, and an invitation can be taken back", async ({ page, browser, playwright }) => {
  const { id } = await workspaceOf(page, playwright);
  const guestAddress = await guestEmail(playwright);
  const link = await invite(page, id, guestAddress);

  // Taken back before it was used: the link says so.
  await page.goto(`/team/${id}`);
  await page.getByRole("button", { name: `Revoke invitation to ${guestAddress}` }).click();
  await expect(page.getByRole("status").filter({ hasText: "was taken back" })).toBeVisible();
  const ben = await another(browser, guestAddress);
  await ben.goto(link);
  await ben.getByRole("button", { name: "Accept invitation" }).click();
  await expect(alertOf(ben)).toContainText("not valid any more");

  // A new one, accepted; then they leave.
  const fresh = await invite(page, id, guestAddress);
  await ben.goto(fresh);
  await ben.getByRole("button", { name: "Accept invitation" }).click();
  await expect(ben).toHaveURL(`/team/${id}`);
  await ben.getByRole("button", { name: "Leave" }).click();
  await expect(ben).toHaveURL(/\/team$/);
  await expect(ben.getByText("You are not in a workspace yet.").or(ben.getByRole("listitem")).first()).toBeVisible();
  await page.reload();
  await expect(page.getByRole("region", { name: "People · 1" })).toBeVisible();
  await ben.context().close();
});

test("ending a workspace takes two presses, and ends it for everyone", async ({ page, browser, playwright }) => {
  const { id, name } = await workspaceOf(page, playwright);
  const guestAddress = await guestEmail(playwright);
  const link = await invite(page, id, guestAddress);
  const ben = await another(browser, guestAddress);
  await ben.goto(link);
  await ben.getByRole("button", { name: "Accept invitation" }).click();
  await expect(ben).toHaveURL(`/team/${id}`);

  await page.goto(`/team/${id}`);
  await page.getByRole("button", { name: `Delete workspace ${name}` }).click();
  await page.getByRole("button", { name: "Keep it" }).click();
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(name);
  await page.getByRole("button", { name: `Delete workspace ${name}` }).click();
  await page.getByRole("button", { name: `Delete for everyone: ${name}` }).click();
  await expect(page).toHaveURL(/\/team$/);
  await expect(page.getByRole("link", { name })).toHaveCount(0);

  await ben.reload();
  await expect(ben.getByRole("heading", { level: 1 })).toHaveText("Not found.");
  await ben.context().close();
});

test("a creator puts a recording in, the team sees it, and only those who may take it out", async ({ page, browser, playwright }) => {
  const { id } = await workspaceOf(page, playwright);
  const guestAddress = await guestEmail(playwright);
  const link = await invite(page, id, guestAddress);
  const ben = await another(browser, guestAddress);
  await ben.goto(link);
  await ben.getByRole("button", { name: "Accept invitation" }).click();
  await expect(ben).toHaveURL(`/team/${id}`);

  const title = unique("Lesson");
  const made = await page.request.post("/v1/recordings", { data: { source: "upload", title } });
  expect(made.status(), await made.text()).toBe(201);
  const { recording } = await made.json();

  try {
    await page.goto(`/team/${id}`);
    const mine = page.getByRole("region", { name: /^Recordings/ });
    await expect(mine).toContainText("Nothing here yet.");
    await mine.getByLabel("Put one of yours in").selectOption({ label: title });
    await mine.getByRole("button", { name: "Put in workspace" }).click();
    await expect(page.getByRole("status").filter({ hasText: `\u201c${title}\u201d is in the workspace.` })).toBeVisible();
    await expect(mine.getByRole("listitem").filter({ hasText: title })).toContainText("You");
    // It is in, so it is no longer offered.
    await expect(mine.locator("option", { hasText: title })).toHaveCount(0);

    // The member sees it, and by whom. They have no library of their own to put one in from, and may not take it out.
    await ben.goto(`/team/${id}`);
    const theirs = ben.getByRole("region", { name: /^Recordings/ });
    const row = theirs.getByRole("listitem").filter({ hasText: title });
    await expect(row).toContainText("Asha Rao");
    await expect(row.getByRole("button", { name: `Take out ${title}` })).toHaveCount(0);
    await expect(theirs.getByLabel("Put one of yours in")).toHaveCount(0);

    // The owner takes it out: the member's next look is empty, and the maker still has it, alone.
    await mine.getByRole("button", { name: `Take out ${title}` }).click();
    await expect(page.getByRole("status").filter({ hasText: "is out of the workspace" })).toBeVisible();
    await expect(mine).toContainText("Nothing here yet.");
    await ben.reload();
    await expect(theirs).toContainText("Nothing here yet.");
    const still = await page.request.get(`/v1/recordings/${recording.id}`);
    expect(still.status()).toBe(200);
    expect((await still.json()).recording.workspace_id ?? null).toBeNull();
  } finally {
    await page.request.delete(`/v1/recordings/${recording.id}`);
    await ben.context().close();
  }
});

test("the team screens pass axe at desktop and phone size", async ({ page, browser, playwright }) => {
  const { id } = await workspaceOf(page, playwright);
  const guestAddress = await guestEmail(playwright);
  const link = await invite(page, id, guestAddress);
  // A recording in it, with a title that is long enough to push a phone screen wider if it is not wrapped.
  const made = await page.request.post("/v1/recordings", { data: { source: "upload", title: unique("Weekly review of the mathematics syllabus for the whole of the ninth grade class") } });
  expect(made.status(), await made.text()).toBe(201);
  const recordingId = (await made.json()).recording.id as string;
  const put = await patiently(() => page.request.put(`/v1/workspaces/${id}/recordings/${recordingId}`));
  expect(put.status(), await put.text()).toBe(204);

  async function axe(where: string) {
    await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
    const result = await page.evaluate(() =>
      // @ts-expect-error axe is added to the page by the script above
      window.axe.run(document, { runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"] } }),
    );
    const summary = result.violations.map((v: { id: string; help: string; nodes: { target: string[] }[] }) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).join(", ")})`);
    expect(summary, where).toEqual([]);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow, `${where} overflows`).toBeLessThanOrEqual(0);
  }

  for (const [size, viewport] of [
    ["desktop", { width: 1280, height: 800 }],
    ["phone", { width: 390, height: 844 }],
  ] as const) {
    await page.setViewportSize(viewport);
    await page.goto("/team");
    await expect(page.getByRole("heading", { level: 1 })).toContainText("workspaces.");
    await axe(`the list at ${size} size`);

    await page.goto(`/team/${id}`);
    await expect(page.getByRole("heading", { name: "Waiting for an answer" })).toBeVisible();
    await axe(`a workspace at ${size} size`);
    await page.getByRole("button", { name: /^Delete workspace / }).click();
    await axe(`asking before ending one at ${size} size`);
    await page.getByRole("button", { name: "Keep it" }).click();

    await page.locator("#invite-email").fill("not an email");
    await page.getByRole("button", { name: "Send invitation" }).click();
    await expect(page.getByText("Enter a valid email address.")).toBeVisible();
    await axe(`a problem on the invite form at ${size} size`);

    const ben = await another(browser, guestAddress);
    await ben.setViewportSize(viewport);
    await ben.goto(link);
    await expect(ben.getByRole("button", { name: "Accept invitation" })).toBeVisible();
    await ben.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
    const result = await ben.evaluate(() =>
      // @ts-expect-error axe is added to the page by the script above
      window.axe.run(document, { runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"] } }),
    );
    expect(result.violations.map((v: { id: string }) => v.id), `the invitation page at ${size} size`).toEqual([]);
    await ben.context().close();
  }
  await page.request.delete(`/v1/recordings/${recordingId}`);
});

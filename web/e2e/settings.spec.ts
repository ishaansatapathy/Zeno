import { expect, test, type Page } from "@playwright/test";

import { PASSWORD, alertOf, confirmedAccount, processingVideo, signIn, signInQuietly } from "./helpers";

const NEW_PASSWORD = "a different long passphrase";

let sharedEmail: Promise<string> | null = null;

/**
 * Signing up is limited to ten a hour from one address, and a suite that made an account for each test would spend
 * the allowance of every spec after it. So the tests here share one account, put back to its name each time, and
 * only the one that changes the password makes its own.
 */
async function settings(page: Page, request: Parameters<typeof confirmedAccount>[0], options: { own?: boolean } = {}) {
  const email = options.own ? await confirmedAccount(request, { name: "Asha Rao" }) : await (sharedEmail ??= confirmedAccount(request, { name: "Asha Rao" }));
  await signInQuietly(page, email);
  if (!options.own) {
    const named = await page.request.patch("/v1/auth/me", { data: { name: "Asha Rao" } });
    expect(named.status()).toBe(200);
  }
  await page.goto("/settings");
  await expect(page.getByRole("heading", { level: 1 })).toContainText("settings.");
  return email;
}

test("the settings say how much is used, and what a deleted video gives back", async ({ page, request }) => {
  await settings(page, request);
  const meter = page.getByRole("progressbar", { name: "Storage used" });
  await expect(meter).toHaveAttribute("aria-valuetext", /^0 bytes of \d/);
  await expect(page.getByText("0 videos counted")).toBeVisible();

  const video = await processingVideo(page);
  await page.reload();
  await expect(meter).toHaveAttribute("aria-valuetext", /^1\.0 MB of /);
  await expect(page.getByText("1 video counted")).toBeVisible();
  // Any use at all shows, however small beside the allowance.
  await expect(meter).toHaveAttribute("aria-valuenow", "1");

  const removed = await page.request.delete(`/v1/videos/${video}`);
  expect(removed.status()).toBe(204);
  await page.reload();
  await expect(meter).toHaveAttribute("aria-valuetext", /^0 bytes of /);
});

test("a full allowance is said plainly, with the way out", async ({ page, request }) => {
  await page.route("**/v1/me/storage", (route) =>
    route.fulfill({ json: { storage: { used_bytes: 25 * 1024 ** 3, quota_bytes: 25 * 1024 ** 3, remaining_bytes: 0, videos: 12 } } }),
  );
  await settings(page, request);
  const meter = page.getByRole("progressbar", { name: "Storage used" });
  await expect(meter).toHaveAttribute("aria-valuenow", "100");
  await expect(meter).toHaveAttribute("aria-valuetext", "25.0 GB of 25.0 GB");
  await expect(page.getByText("12 videos counted")).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "Your storage is full" })).toContainText("still kept on your computer");
  await page.getByRole("link", { name: "Delete what you no longer need" }).click();
  await expect(page).toHaveURL(/\/library$/);
});

test("when the numbers cannot be had, it says so and tries again on request", async ({ page, request }) => {
  let failing = true;
  await page.route("**/v1/me/storage", (route) =>
    failing ? route.fulfill({ status: 503, json: { error: { code: "unavailable", message: "x" } } }) : route.continue(),
  );
  await settings(page, request);
  await expect(alertOf(page)).toContainText("Zeno is busy right now");
  failing = false;
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByRole("progressbar", { name: "Storage used" })).toBeVisible();
  await expect(alertOf(page)).toHaveCount(0);
});

test("a person renames themselves, and the new name is what is kept", async ({ page, request }) => {
  await settings(page, request);
  const header = page.getByRole("link", { name: /^Settings, / });
  await expect(header).toContainText("Asha Rao");

  await page.getByRole("textbox", { name: "Name", exact: true }).fill("Asha Roy");
  await page.getByRole("button", { name: "Save name" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Name saved." })).toBeVisible();
  // The bar already says it, without asking again.
  await expect(page.getByRole("link", { name: "Settings, Asha Roy" })).toBeVisible();

  await page.reload();
  await expect(page.getByRole("textbox", { name: "Name", exact: true })).toHaveValue("Asha Roy");
  const me = await (await page.request.get("/v1/auth/me")).json();
  expect(me.user.name).toBe("Asha Roy");
});

test("a name that cannot be kept is said so before anything is sent", async ({ page, request }) => {
  await settings(page, request);
  let sent = 0;
  await page.route("**/v1/auth/me", (route) => {
    if (route.request().method() === "PATCH") sent += 1;
    return route.continue();
  });
  await page.getByRole("textbox", { name: "Name", exact: true }).fill("A");
  await page.getByRole("button", { name: "Save name" }).click();
  await expect(page.getByText("Name must be 2 to 80 characters.")).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Name", exact: true })).toBeFocused();
  expect(sent).toBe(0);
  // The same name again is not a change.
  await page.getByRole("textbox", { name: "Name", exact: true }).fill("Asha Rao");
  await page.getByRole("button", { name: "Save name" }).click();
  await expect(page.getByRole("status").filter({ hasText: "That is already your name." })).toBeVisible();
  expect(sent).toBe(0);
});

test("changing the password asks for the old one, then signs the person out everywhere", async ({ page, request, browser }) => {
  const email = await settings(page, request, { own: true });
  // Somewhere else the same account is signed in.
  const elsewhere = await browser.newContext({ baseURL: "http://localhost:3000" });
  const other = await elsewhere.newPage();
  await signInQuietly(other, email);
  expect((await other.request.get("/v1/auth/me")).status()).toBe(200);

  // Typed wrongly: told on the field, and still signed in.
  await page.getByLabel("Current password").fill("not the password at all");
  await page.getByLabel("New password", { exact: true }).fill(NEW_PASSWORD);
  await page.getByLabel("Type the new one again").fill(NEW_PASSWORD);
  await page.getByRole("button", { name: "Change password" }).click();
  await expect(page.getByText("That is not your current password.")).toBeVisible();
  await expect(page.getByLabel("Current password")).toBeFocused();
  await expect(page).toHaveURL(/\/settings$/);

  // The form's own checks come first and send nothing.
  await page.getByLabel("Current password").fill(PASSWORD);
  await page.getByLabel("Type the new one again").fill("something else entirely");
  await page.getByRole("button", { name: "Change password" }).click();
  await expect(page.getByText("The two passwords don't match.")).toBeVisible();
  await page.getByLabel("New password", { exact: true }).fill(PASSWORD);
  await page.getByLabel("Type the new one again").fill(PASSWORD);
  await page.getByRole("button", { name: "Change password" }).click();
  await expect(page.getByText("Choose a password you are not using now.")).toBeVisible();

  await page.getByLabel("New password", { exact: true }).fill(NEW_PASSWORD);
  await page.getByLabel("Type the new one again").fill(NEW_PASSWORD);
  await page.getByRole("button", { name: "Change password" }).click();
  await expect(page).toHaveURL(/\/login\?arrival=password-changed$/);
  await expect(page.getByRole("status")).toContainText("signed out everywhere");

  // Here, and the other place, nobody is signed in any more; the old password is dead and the new one is not.
  expect((await page.request.get("/v1/auth/me")).status()).toBe(401);
  expect((await other.request.get("/v1/auth/me")).status()).toBe(401);
  await elsewhere.close();
  await signIn(page, email);
  await expect(alertOf(page)).toContainText("don't match");
  await signIn(page, email, NEW_PASSWORD);
  await expect(page).toHaveURL(/\/library$/);
});

test("the settings pass axe at desktop and phone size, with and without something to say", async ({ page, request }) => {
  await settings(page, request);

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

  for (const [name, size] of [
    ["desktop", { width: 1280, height: 800 }],
    ["phone", { width: 390, height: 844 }],
  ] as const) {
    await page.setViewportSize(size);
    await page.goto("/settings");
    await expect(page.getByRole("progressbar", { name: "Storage used" })).toBeVisible();
    await axe(`the settings at ${name} size`);

    await page.getByRole("textbox", { name: "Name", exact: true }).fill("A");
    await page.getByRole("button", { name: "Save name" }).click();
    await page.getByLabel("Current password").fill("x");
    await page.getByLabel("New password", { exact: true }).fill("short");
    await page.getByRole("button", { name: "Change password" }).click();
    await expect(page.getByText("Password must be at least 10 characters.")).toBeVisible();
    await axe(`the settings with problems at ${name} size`);
  }
});

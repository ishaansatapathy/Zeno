import { expect, test } from "@playwright/test";

import { PASSWORD, alertOf, confirmedAccount, sharedAccount, signIn, unique } from "./helpers";
import { mailedCount, mailedLink } from "./mail";

const NEW_PASSWORD = "a different long passphrase";

test("a new person registers, confirms their email, signs out and signs back in", async ({ page }) => {
  const email = unique("asha");

  await page.goto("/register");
  await page.getByLabel("Your name").fill("Asha Rao");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByRole("status")).toContainText("a link is on its way");

  // The email's link opens a page that waits to be asked: nothing is used up by arriving.
  await page.goto(await mailedLink(email, "verify-email"));
  await expect(page.getByRole("heading", { level: 1 })).toContainText("Confirm your");
  await page.getByRole("button", { name: "Confirm email" }).click();
  await expect(page).toHaveURL(/\/library$/);
  await expect(page.getByText(`Signed in as ${email}`)).toBeVisible();

  // The session survives a reload.
  await page.reload();
  await expect(page.getByText(`Signed in as ${email}`)).toBeVisible();

  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/login\?arrival=signed-out$/);
  await expect(page.getByRole("status")).toContainText("You're signed out.");

  // Signed out means signed out: the library sends the person to sign in.
  await page.goto("/library");
  await expect(page).toHaveURL(/\/login\?next=%2Flibrary$/);

  await signIn(page, email);
  await expect(page).toHaveURL(/\/library$/);
  await expect(page.getByText(`Signed in as ${email}`)).toBeVisible();
});

test("the form says what is wrong before asking the server, and puts the cursor there", async ({ page }) => {
  await page.goto("/register");
  await page.getByRole("button", { name: "Create account" }).click();

  await expect(page.getByText("Name must be 2 to 80 characters.")).toBeVisible();
  await expect(page.getByText("Email is required.")).toBeVisible();
  await expect(page.getByText("Password must be at least 10 characters.")).toBeVisible();
  await expect(page.getByLabel("Your name")).toBeFocused();
  await expect(page.getByLabel("Your name")).toHaveAttribute("aria-invalid", "true");
  await expect(page.getByLabel("Your name")).toHaveAccessibleDescription("Name must be 2 to 80 characters.");

  // Fixing a field and sending again clears its error.
  await page.getByLabel("Your name").fill("Asha");
  await page.getByLabel("Email").fill("asha@example.com");
  await page.getByLabel("Password").fill("short");
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByText("Name must be 2 to 80 characters.")).toHaveCount(0);
  await expect(page.getByLabel("Password")).toBeFocused();
});

test("the server's own objections show against the field they are about", async ({ page }) => {
  await page.goto("/register");
  await page.getByLabel("Your name").fill("Asha Rao");
  await page.getByLabel("Email").fill("asha@example.com");
  // Long enough for the form, but one of the most common passwords there is.
  await page.getByLabel("Password").fill("password123456");
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.locator("#password-error")).toBeVisible();
  await expect(page.getByLabel("Password")).toBeFocused();
});

test("signing in with the wrong password, or before confirming, explains and offers the way forward", async ({ page }) => {
  const email = unique("asha");
  await page.goto("/register");
  await page.getByLabel("Your name").fill("Asha Rao");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByRole("status")).toContainText("a link is on its way");
  await mailedLink(email, "verify-email");

  await page.goto("/login");
  await signIn(page, email);
  await expect(alertOf(page)).toContainText("Confirm your email address first");

  // "Send it again" really sends another, and then waits before it can be pressed again.
  const before = mailedCount(email, "verify-email");
  await page.getByRole("button", { name: "Send it again" }).click();
  await expect(page.getByRole("status")).toContainText("Sent again");
  await expect(page.getByRole("button", { name: /Send again in \d+s/ })).toBeDisabled();
  expect(await mailedLink(email, "verify-email", before)).toMatch(/^\/verify-email\//);

  const other = await page.context().newPage();
  await other.goto("/login");
  await other.getByLabel("Email").fill("nobody@example.com");
  await other.getByLabel("Password").fill("not the password");
  await other.getByRole("button", { name: "Sign in" }).click();
  await expect(alertOf(other)).toContainText("don't match");
});

test("a used or made-up confirmation link says so and offers a new one", async ({ page }) => {
  await page.goto("/verify-email/not-a-real-token");
  await page.getByRole("button", { name: "Confirm email" }).click();
  await expect(alertOf(page)).toContainText("invalid or has expired");
  await page.getByRole("link", { name: "Get a new link" }).click();
  await expect(page).toHaveURL(/\/verify-email$/);
});

test("a forgotten password is reset from the emailed link, and the old one stops working", async ({ page, request }) => {
  const email = await confirmedAccount(request);

  await page.goto("/forgot-password");
  await page.getByLabel("Email").fill(email);
  await page.getByRole("button", { name: "Send link" }).click();
  await expect(page.getByRole("status")).toContainText("we've sent instructions");

  await page.goto(await mailedLink(email, "reset-password"));
  await page.getByLabel("New password").fill(NEW_PASSWORD);
  await page.getByLabel("Type it again").fill("something else entirely");
  await page.getByRole("button", { name: "Set new password" }).click();
  await expect(page.getByText("The two passwords don't match.")).toBeVisible();

  await page.getByLabel("Type it again").fill(NEW_PASSWORD);
  await page.getByRole("button", { name: "Set new password" }).click();
  await expect(page).toHaveURL(/\/login\?arrival=reset$/);
  await expect(page.getByRole("status")).toContainText("Your password is updated");

  await signIn(page, email, PASSWORD);
  await expect(alertOf(page)).toContainText("don't match");
  await signIn(page, email, NEW_PASSWORD);
  await expect(page).toHaveURL(/\/library$/);
});

test("an emailed reset link works once", async ({ page, request }) => {
  const email = await confirmedAccount(request);
  await request.post("/v1/auth/forgot-password", { data: { email } });
  const link = await mailedLink(email, "reset-password");

  await page.goto(link);
  await page.getByLabel("New password").fill(NEW_PASSWORD);
  await page.getByLabel("Type it again").fill(NEW_PASSWORD);
  await page.getByRole("button", { name: "Set new password" }).click();
  await expect(page).toHaveURL(/\/login\?arrival=reset$/);

  await page.goto(link);
  await page.getByLabel("New password").fill("yet another long passphrase");
  await page.getByLabel("Type it again").fill("yet another long passphrase");
  await page.getByRole("button", { name: "Set new password" }).click();
  await expect(alertOf(page)).toContainText("invalid or has expired");
  await expect(page.getByRole("link", { name: "Ask for a new link" })).toBeVisible();
});

test("the library asks for a sign-in and brings the person back to it; a link to another site is ignored", async ({ page, playwright }) => {
  const email = await sharedAccount(playwright);

  await page.goto("/library");
  await expect(page).toHaveURL(/\/login\?next=%2Flibrary$/);
  await signIn(page, email);
  await expect(page).toHaveURL(/\/library$/);

  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/login/);

  await page.goto("/login?next=//evil.example/steal");
  await signIn(page, email);
  await expect(page).toHaveURL(/localhost:3000\/library$/);
});

test("a session that lapses is renewed without the person noticing", async ({ page, context, playwright }) => {
  const email = await sharedAccount(playwright);
  await page.goto("/login");
  await signIn(page, email);
  await expect(page).toHaveURL(/\/library$/);

  // The access cookie lasts minutes. Take it away, as time would, and look again.
  await context.clearCookies({ name: "zeno_at" });
  const refreshes: string[] = [];
  page.on("request", (req) => {
    if (req.url().endsWith("/v1/auth/refresh")) refreshes.push(req.url());
  });
  await page.reload();
  await expect(page.getByText(`Signed in as ${email}`)).toBeVisible();
  expect(refreshes).toHaveLength(1);
});

test("a person whose session cannot be renewed is sent to sign in", async ({ page, context, playwright }) => {
  const email = await sharedAccount(playwright);
  await page.goto("/login");
  await signIn(page, email);
  await expect(page).toHaveURL(/\/library$/);

  await context.clearCookies();
  await page.reload();
  await expect(page).toHaveURL(/\/login\?next=%2Flibrary$/);
});

test("pages that carry a one-time token are kept out of caches and referrers", async ({ page }) => {
  for (const path of ["/verify-email/abc", "/reset-password/abc"]) {
    const response = await page.goto(path);
    expect(response?.headers()["referrer-policy"], path).toBe("no-referrer");
    expect(response?.headers()["cache-control"], path).toContain("no-store");
  }
  // Nothing on an account screen belongs in a search engine.
  await page.goto("/login");
  await expect(page.locator('meta[name="robots"]')).toHaveAttribute("content", /noindex/);
});

test.describe("accessibility", () => {
  const screens = ["/login", "/register", "/forgot-password", "/verify-email", "/verify-email/abc", "/reset-password/abc"];

  for (const [name, size] of [
    ["desktop", { width: 1280, height: 800 }],
    ["phone", { width: 390, height: 844 }],
  ] as const) {
    test(`every account screen passes axe at ${name} size`, async ({ page }) => {
      await page.setViewportSize(size);
      for (const path of screens) {
        await page.goto(path);
        await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
        const result = await page.evaluate(() =>
          // @ts-expect-error axe is added to the page by the script above
          window.axe.run(document, { runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"] } }),
        );
        const summary = result.violations.map((v: { id: string; help: string; nodes: { target: string[] }[] }) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).join(", ")})`);
        expect(summary, `${path} at ${name}`).toEqual([]);
        // No sideways scrolling on a phone.
        const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
        expect(overflow, `${path} overflows at ${name}`).toBeLessThanOrEqual(0);
      }
    });
  }

  test("the signed-in library passes axe", async ({ page, playwright }) => {
    const email = await sharedAccount(playwright);
    await page.goto("/login");
    await signIn(page, email);
    await expect(page).toHaveURL(/\/library$/);
    await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
    const result = await page.evaluate(() =>
      // @ts-expect-error axe is added to the page by the script above
      window.axe.run(document, { runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"] } }),
    );
    expect(result.violations.map((v: { id: string }) => v.id)).toEqual([]);
  });
});

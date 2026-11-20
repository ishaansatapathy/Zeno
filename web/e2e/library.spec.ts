import { expect, test, type Page, type PlaywrightWorkerArgs } from "@playwright/test";

import { alertOf, emptyLibrary, seedRecording, sharedAccount, signInQuietly } from "./helpers";

/** The shared creator, signed in, looking at a library with nothing in it. */
async function creator(page: Page, playwright: PlaywrightWorkerArgs["playwright"]) {
  const email = await sharedAccount(playwright);
  await signInQuietly(page, email);
  await emptyLibrary(page);
  return email;
}

const rowOf = (page: Page, title: string) => page.getByRole("listitem").filter({ has: page.getByRole("heading", { name: title, exact: true }) });

test("an empty library says so, and a new recording shows what it is", async ({ page, playwright }) => {
  await creator(page, playwright);
  await page.goto("/library");
  await expect(page.getByText("Nothing here yet")).toBeVisible();

  await seedRecording(page, "Week 3");
  await page.reload();
  const row = rowOf(page, "Week 3");
  await expect(row).toBeVisible();
  await expect(row).toContainText("Extension");
  await expect(row).toContainText("Capturing");
  await expect(row).toContainText("Private");
  await expect(row).toContainText("0 views");
});

test("a recording is renamed in place, and the new name is what is kept", async ({ page, playwright }) => {
  await creator(page, playwright);
  const { id } = await seedRecording(page, "Week 3");
  await page.goto("/library");

  await page.getByRole("button", { name: "Rename Week 3" }).click();
  const input = page.getByLabel("Title");
  await expect(input).toBeFocused();
  await input.fill("Week 4: loops");
  await input.press("Enter");
  await expect(page.getByRole("status").filter({ hasText: "Renamed to Week 4: loops." })).toBeVisible();
  await expect(rowOf(page, "Week 4: loops")).toBeVisible();
  // Back where the person was: on the button that opened the editor.
  await expect(page.getByRole("button", { name: "Rename Week 4: loops" })).toBeFocused();

  await page.reload();
  await expect(rowOf(page, "Week 4: loops")).toBeVisible();
  const kept = await (await page.request.get(`/v1/recordings/${id}`)).json();
  expect(kept.recording.title).toBe("Week 4: loops");
  expect(kept.recording.version).toBe(2);
});

test("renaming can be cancelled, and refuses a title that cannot be kept", async ({ page, playwright }) => {
  await creator(page, playwright);
  await seedRecording(page, "Week 3");
  await page.goto("/library");

  await page.getByRole("button", { name: "Rename Week 3" }).click();
  await page.getByLabel("Title").fill("Something else");
  await page.getByLabel("Title").press("Escape");
  await expect(rowOf(page, "Week 3")).toBeVisible();
  await expect(page.getByRole("button", { name: "Rename Week 3" })).toBeFocused();

  await page.getByRole("button", { name: "Rename Week 3" }).click();
  await page.getByLabel("Title").fill("   ");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Give it a title.")).toBeVisible();
  await expect(page.getByLabel("Title")).toHaveAttribute("aria-invalid", "true");

  await page.getByLabel("Title").fill("x".repeat(121));
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("A title can be at most 120 characters.")).toBeVisible();

  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(rowOf(page, "Week 3")).toBeVisible();
});

test("a recording changed elsewhere is reloaded instead of overwritten", async ({ page, playwright }) => {
  await creator(page, playwright);
  const { id, version } = await seedRecording(page, "Week 3");
  await page.goto("/library");

  // Another tab or device renames it while this page still shows the old name: the page must have
  // shown it first, or there is nothing stale to speak of.
  await expect(rowOf(page, "Week 3")).toBeVisible();
  const elsewhere = await page.request.patch(`/v1/recordings/${id}`, { data: { version, title: "Renamed elsewhere" } });
  expect(elsewhere.status()).toBe(200);

  await page.getByRole("button", { name: "Rename Week 3" }).click();
  await page.getByLabel("Title").fill("My rename");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("status").filter({ hasText: "changed elsewhere" })).toBeVisible();
  await expect(rowOf(page, "Renamed elsewhere")).toBeVisible();

  // Now it is current, so the second try works.
  await page.getByRole("button", { name: "Rename Renamed elsewhere" }).click();
  await page.getByLabel("Title").fill("My rename");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(rowOf(page, "My rename")).toBeVisible();
});

test("deleting asks first, defaults to keeping, and removes the recording for good", async ({ page, playwright }) => {
  await creator(page, playwright);
  const { id } = await seedRecording(page, "Week 3");
  await seedRecording(page, "Week 2");
  await page.goto("/library");

  await page.getByRole("button", { name: "Delete Week 3" }).click();
  await expect(page.getByRole("group", { name: "Delete Week 3?" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Keep it" })).toBeFocused();
  await page.getByRole("button", { name: "Keep it" }).click();
  await expect(rowOf(page, "Week 3")).toBeVisible();

  await page.getByRole("button", { name: "Delete Week 3" }).click();
  await page.getByRole("button", { name: "Delete", exact: true }).click();
  await expect(page.getByRole("status").filter({ hasText: "Deleted Week 3." })).toBeVisible();
  await expect(rowOf(page, "Week 3")).toHaveCount(0);
  await expect(rowOf(page, "Week 2")).toBeVisible();

  expect((await page.request.get(`/v1/recordings/${id}`)).status()).toBe(404);
  await page.reload();
  await expect(rowOf(page, "Week 3")).toHaveCount(0);
});

test("copying a link makes one the first time, and hands out the same one after", async ({ page, playwright, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await creator(page, playwright);
  const { id } = await seedRecording(page, "Week 3");
  await page.goto("/library");
  const row = rowOf(page, "Week 3");
  await expect(row).toContainText("Private");

  await page.getByRole("button", { name: "Copy link for Week 3" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Link copied." })).toBeVisible();
  const first = await page.evaluate(() => navigator.clipboard.readText());
  expect(first).toMatch(/^http:\/\/localhost:3000\/s\/[A-Za-z0-9_-]{22}$/);
  // Sharing a recording is what makes it visible to people with the link, and the row says so.
  await expect(row).toContainText("Anyone with the link");

  // A second press reuses the link; it does not pile up new ones.
  await expect(page.getByRole("button", { name: "Copy link for Week 3" })).toBeEnabled();
  await page.getByRole("button", { name: "Copy link for Week 3" }).click();
  await expect(page.getByRole("button", { name: "Copy link for Week 3" })).toContainText("Copied");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(first);
  const links = await (await page.request.get(`/v1/recordings/${id}/links`)).json();
  expect(links.links).toHaveLength(1);

  // Making the link edited the recording; the row has the new version, so renaming still works.
  await page.getByRole("button", { name: "Rename Week 3" }).click();
  await page.getByLabel("Title").fill("Week 3, shared");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(rowOf(page, "Week 3, shared")).toBeVisible();
});

test("a revoked link is replaced by a new one when copying again", async ({ page, playwright, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await creator(page, playwright);
  const { id } = await seedRecording(page, "Week 3");
  await page.goto("/library");
  await page.getByRole("button", { name: "Copy link for Week 3" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Link copied." })).toBeVisible();
  const first = await page.evaluate(() => navigator.clipboard.readText());

  const listed = await (await page.request.get(`/v1/recordings/${id}/links`)).json();
  const revoked = await page.request.delete(`/v1/recordings/${id}/links/${listed.links[0].id}`);
  expect(revoked.status()).toBe(204);

  await expect(page.getByRole("button", { name: "Copy link for Week 3" })).toContainText("Copy link");
  await page.getByRole("button", { name: "Copy link for Week 3" }).click();
  await expect(page.getByRole("button", { name: "Copy link for Week 3" })).toContainText("Copied");
  const second = await page.evaluate(() => navigator.clipboard.readText());
  expect(second).not.toBe(first);
});

test("a long library is read a page at a time, newest first", async ({ page, playwright }) => {
  await creator(page, playwright);
  for (let n = 1; n <= 23; n += 1) await seedRecording(page, `Item ${String(n).padStart(2, "0")}`);
  await page.goto("/library");

  const titles = page.getByRole("listitem").getByRole("heading");
  await expect(titles).toHaveCount(20);
  await expect(titles.first()).toHaveText("Item 23");
  await expect(titles.last()).toHaveText("Item 04");

  await page.getByRole("button", { name: "Load more" }).click();
  await expect(titles).toHaveCount(23);
  await expect(titles.last()).toHaveText("Item 01");
  await expect(page.getByRole("button", { name: "Load more" })).toHaveCount(0);
});

test("the library can be narrowed to how a recording was made", async ({ page, playwright }) => {
  await creator(page, playwright);
  await seedRecording(page, "From the extension", "extension");
  await seedRecording(page, "From an upload", "upload");
  await page.goto("/library");
  await expect(page.getByRole("listitem")).toHaveCount(2);

  await page.getByRole("button", { name: "Uploads" }).click();
  await expect(page.getByRole("button", { name: "Uploads" })).toHaveAttribute("aria-pressed", "true");
  await expect(rowOf(page, "From an upload")).toBeVisible();
  await expect(rowOf(page, "From the extension")).toHaveCount(0);

  await page.getByRole("button", { name: "Studio" }).click();
  await expect(page.getByText("No recordings made this way.")).toBeVisible();

  await page.getByRole("button", { name: "All" }).click();
  await expect(page.getByRole("listitem")).toHaveCount(2);
});

test("an account that only watches is told it has no library, not shown an error", async ({ page, playwright }) => {
  const email = await sharedAccount(playwright, "student");
  await signInQuietly(page, email);
  await page.goto("/library");
  await expect(page.getByText("has no library")).toBeVisible();
  await expect(alertOf(page)).toHaveCount(0);
});

test("the library passes axe at desktop and phone size, in every state a row can be in", async ({ page, playwright }) => {
  await creator(page, playwright);
  await seedRecording(page, "Week 3");
  await seedRecording(page, "Week 2", "upload");

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
    await page.goto("/library");
    await expect(rowOf(page, "Week 3")).toBeVisible();
    await axe(`the list at ${name} size`);

    await page.getByRole("button", { name: "Rename Week 3" }).click();
    await axe(`renaming at ${name} size`);
    await page.getByLabel("Title").press("Escape");

    await page.getByRole("button", { name: "Delete Week 3" }).click();
    await axe(`confirming a delete at ${name} size`);
    await page.getByRole("button", { name: "Keep it" }).click();
  }
});

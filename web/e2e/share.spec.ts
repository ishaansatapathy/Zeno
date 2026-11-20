import { expect, test, type Browser, type Page } from "@playwright/test";

import { alertOf, attachVideo, processingVideo, revokeLink, seedRecording, sharedAccount, shareLink, signInQuietly } from "./helpers";
import { apiRequestsTo } from "./log";

const BASE = "http://localhost:3000";

/** Someone with no account and no cookies, opening a link. */
async function visitor(browser: Browser) {
  const context = await browser.newContext({ baseURL: BASE });
  return { context, page: await context.newPage() };
}

/** The shared creator, signed in. */
async function owner(page: Page, playwright: Parameters<typeof sharedAccount>[0]) {
  await signInQuietly(page, await sharedAccount(playwright));
}

/** A recording with the given title, and a link to it. `video: "processing"` gives it an upload the worker has not finished. */
async function shared(page: Page, title: string, options: { video?: "processing"; password?: string } = {}) {
  const recording = await seedRecording(page, title);
  if (options.video === "processing") await attachVideo(page, recording.id, await processingVideo(page));
  const link = await shareLink(page, recording.id, options.password ? { password: options.password } : {});
  return { recording, ...link };
}

const SHARE_JSON = (code: string, over: Record<string, unknown> = {}) => ({ share: { code, requires_password: false, title: "Week 3", status: "ready", duration_seconds: 125, ...over } });

test("a link that does not exist says so, and is a 404", async ({ browser }) => {
  const { context, page } = await visitor(browser);
  for (const path of [`/s/${"A".repeat(22)}`, "/s/short", `/s/${"A".repeat(40)}`]) {
    const response = await page.goto(path);
    expect(response?.status(), path).toBe(404);
    await expect(page.getByRole("heading", { level: 1 })).toContainText("find");
  }
  await context.close();
});

test("a link that has closed says so, and tells search engines and referrers nothing", async ({ page, browser, playwright }) => {
  await owner(page, playwright);
  const s = await shared(page, "Week 3");
  await revokeLink(page, s.recording.id, s.id);

  const { context, page: seen } = await visitor(browser);
  const response = await seen.goto(`/s/${s.code}`);
  await expect(seen.getByRole("heading", { level: 1 })).toContainText("no longer available");
  expect(response?.headers()["referrer-policy"]).toBe("no-referrer");
  expect(response?.headers()["x-robots-tag"]).toContain("noindex");
  await expect(seen.locator('meta[name="robots"]')).toHaveAttribute("content", /noindex/);
  await context.close();
});

test("a recording that is not finished says so", async ({ page, browser, playwright }) => {
  await owner(page, playwright);
  const s = await shared(page, "Week 3");

  const { context, page: seen } = await visitor(browser);
  await seen.goto(`/s/${s.code}`);
  await expect(seen.getByRole("heading", { level: 1 })).toContainText("recorded");
  await expect(seen.getByText("Week 3", { exact: true })).toBeVisible();
  await context.close();
});

test("a video being prepared says so, and the page turns into the player by itself when it is ready", async ({ page, browser, playwright }) => {
  test.setTimeout(90_000);
  await owner(page, playwright);
  const s = await shared(page, "Week 3", { video: "processing" });

  const { context, page: seen } = await visitor(browser);
  await seen.goto(`/s/${s.code}`);
  await expect(seen.getByRole("heading", { level: 1 })).toContainText("ready");
  await expect(seen.getByRole("status")).toContainText("update by itself");
  await expect(seen.getByRole("button", { name: /^Play/ })).toHaveCount(0);

  // The worker finishes: the next time the page asks, the answer is "ready".
  let asked = 0;
  await seen.route(`**/v1/s/${s.code}`, async (route) => {
    asked += 1;
    await route.fulfill({ json: SHARE_JSON(s.code) });
  });
  await expect(seen.getByRole("button", { name: "Play Week 3" })).toBeVisible({ timeout: 20_000 });
  expect(asked).toBeGreaterThan(0);
  await context.close();
});

test("a protected link reveals nothing about the video until the password is given", async ({ page, browser, playwright, request }) => {
  await owner(page, playwright);
  const s = await shared(page, "The secret title", { video: "processing", password: "open sesame" });

  // Not in the page the server sends, and not in the preview a chat app would make.
  const html = await (await request.get(`/s/${s.code}`)).text();
  expect(html).not.toContain("The secret title");
  expect(html).toContain("A protected video");

  const { context, page: seen } = await visitor(browser);
  await seen.goto(`/s/${s.code}`);
  await expect(seen.getByRole("heading", { level: 1 })).toHaveText("A protected video");

  await seen.getByRole("button", { name: "Unlock and play" }).click();
  await expect(seen.getByText("Enter the password to watch.")).toBeVisible();

  await seen.getByLabel("Password").fill("open sesamE");
  await seen.getByRole("button", { name: "Unlock and play" }).click();
  await expect(seen.getByText("That password isn't right.")).toBeVisible();
  await expect(seen.getByLabel("Password")).toBeFocused();

  // The right password, for a video that is still being prepared: told so, and can try again.
  await seen.getByLabel("Password").fill("open sesame");
  await seen.getByRole("button", { name: "Unlock and play" }).click();
  await expect(seen.getByText("isn't ready to watch yet")).toBeVisible();
  await seen.getByRole("button", { name: "Try again" }).click();
  await expect(seen.getByLabel("Password")).toBeVisible();

  // A visitor is never given a cookie.
  expect(await context.cookies()).toEqual([]);
  await context.close();
});

test("a link's preview names the video, and a protected link's does not", async ({ page, playwright, request }) => {
  await owner(page, playwright);
  const open = await shared(page, "Week 3 <b>& more</b>");
  const locked = await shared(page, "Do not leak this", { password: "open sesame" });

  const html = await (await request.get(`/s/${open.code}`)).text();
  expect(html).toContain('property="og:title" content="Week 3 &lt;b&gt;&amp; more&lt;/b&gt;"');
  expect(html).toContain('name="twitter:card" content="summary_large_image"');
  expect(html).toContain('property="og:site_name" content="Zeno"');
  const image = html.match(/property="og:image" content="([^"]+)"/)?.[1];
  expect(image, "an og:image tag").toBeTruthy();

  // The image is a real, non-trivial PNG.
  const picture = await request.get(new URL(image!).pathname);
  expect(picture.status()).toBe(200);
  expect(picture.headers()["content-type"]).toContain("image/png");
  expect((await picture.body()).length).toBeGreaterThan(5_000);

  const hidden = await (await request.get(`/s/${locked.code}`)).text();
  expect(hidden).not.toContain("Do not leak this");
  expect(hidden).toContain('property="og:title" content="A protected video"');
  const hiddenImage = hidden.match(/property="og:image" content="([^"]+)"/)?.[1];
  const hiddenPicture = await request.get(new URL(hiddenImage!).pathname);
  expect(hiddenPicture.status()).toBe(200);
});

test("a page that many people open costs the API one request, not one each", async ({ page, playwright, request }) => {
  await owner(page, playwright);
  const s = await shared(page, "Week 3");

  const first = await request.get(`/s/${s.code}`);
  // Only a production server reuses rendered pages; `next dev` renders every time.
  test.skip(!first.headers()["x-nextjs-cache"], "this server renders every request (it is not `next start`)");
  expect(first.headers()["cache-control"]).toContain("s-maxage=15");

  for (let n = 0; n < 24; n += 1) expect((await request.get(`/s/${s.code}`)).status()).toBe(200);
  const asked = apiRequestsTo(`/v1/s/${s.code}`);
  expect(asked, "API requests for 25 page views").toBeLessThanOrEqual(1);
});

test.describe("the player, with the API's answers played back", () => {
  /** A page for a ready video, reached the way a real one is: preparing, then ready. */
  async function readyPage(page: Page, browser: Browser, playwright: Parameters<typeof sharedAccount>[0]) {
    await owner(page, playwright);
    const s = await shared(page, "Week 3", { video: "processing" });
    const seen = await visitor(browser);
    await seen.page.route(`**/v1/s/${s.code}`, (route) => route.fulfill({ json: SHARE_JSON(s.code) }));
    await seen.page.goto(`/s/${s.code}`);
    await expect(seen.page.getByRole("button", { name: "Play Week 3" })).toBeVisible({ timeout: 20_000 });
    return { ...seen, code: s.code };
  }

  const play = (code: string, body: unknown, status = 201, headers: Record<string, string> = {}) => ({ code, body, status, headers });

  test("pressing play starts a viewing, fetches the playlist, and shows a moving watermark", async ({ page, browser, playwright }) => {
    test.setTimeout(90_000);
    const seen = await readyPage(page, browser, playwright);
    const requested: string[] = [];
    await seen.page.route(`**/v1/s/${seen.code}/play`, (route) =>
      route.fulfill({
        status: 201,
        json: { playback: { session_id: "11111111-1111-4111-8111-111111111111", playlist_url: "/v1/play/test/master.m3u8?t=abc", expires_at: new Date(Date.now() + 3_600_000).toISOString(), watermark: { text: "Shared view · TEST1234", session_code: "TEST1234" } } },
      }),
    );
    await seen.page.route("**/v1/play/test/**", async (route) => {
      const url = new URL(route.request().url());
      requested.push(url.pathname);
      if (url.pathname.endsWith("master.m3u8")) {
        return route.fulfill({ contentType: "application/vnd.apple.mpegurl", body: "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-STREAM-INF:BANDWIDTH=300000,RESOLUTION=256x144\n144p.m3u8\n" });
      }
      if (url.pathname.endsWith("144p.m3u8")) {
        return route.fulfill({ contentType: "application/vnd.apple.mpegurl", body: "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:6,\nseg0.ts\n#EXT-X-ENDLIST\n" });
      }
      // The segment never arrives in time: the viewing stays on screen long enough to look at.
      await new Promise((resolve) => setTimeout(resolve, 4_000));
      return route.fulfill({ status: 404, body: "gone" });
    });

    await seen.page.getByRole("button", { name: "Play Week 3" }).click();
    await expect(seen.page.getByLabel("Week 3", { exact: true })).toBeVisible();
    await expect(seen.page.getByText("Shared view · TEST1234")).toBeVisible();
    await expect.poll(() => requested.some((path) => path.endsWith("master.m3u8"))).toBe(true);
    await expect.poll(() => requested.some((path) => path.endsWith("144p.m3u8"))).toBe(true);

    // The server refuses the segment (the viewing is over, or the link closed): say so, offer play again.
    await expect(alertOf(seen.page)).toContainText("This viewing has ended", { timeout: 20_000 });
    await expect(seen.page.getByRole("button", { name: "Play Week 3" })).toBeVisible();
    await seen.context.close();
  });

  test("a viewing that runs out says so instead of stalling", async ({ page, browser, playwright }) => {
    const seen = await readyPage(page, browser, playwright);
    await seen.page.route(`**/v1/s/${seen.code}/play`, (route) =>
      route.fulfill({
        status: 201,
        json: { playback: { session_id: "22222222-2222-4222-8222-222222222222", playlist_url: "/v1/play/slow/master.m3u8?t=abc", expires_at: new Date(Date.now() + 1_500).toISOString(), watermark: { text: "Shared view · SLOW0000", session_code: "SLOW0000" } } },
      }),
    );
    await seen.page.route("**/v1/play/slow/**", () => new Promise(() => undefined));
    await seen.page.getByRole("button", { name: "Play Week 3" }).click();
    await expect(alertOf(seen.page)).toContainText("This viewing has ended", { timeout: 10_000 });
    await seen.context.close();
  });

  for (const [name, answer, expected] of [
    ["too many viewings from one address", play("", { error: { code: "too_many_requests", message: "m" } }, 429, { "retry-after": "90" }), "Try again in 2 minutes"],
    ["the server failing", play("", { error: { code: "internal_error", message: "m" } }, 500), "Something went wrong on our side"],
  ] as const) {
    test(`${name} is a sentence and a button to try again`, async ({ page, browser, playwright }) => {
      const seen = await readyPage(page, browser, playwright);
      await seen.page.route(`**/v1/s/${seen.code}/play`, (route) => route.fulfill({ status: answer.status, json: answer.body, headers: answer.headers }));
      await seen.page.getByRole("button", { name: "Play Week 3" }).click();
      await expect(alertOf(seen.page)).toContainText(expected);
      await expect(seen.page.getByRole("button", { name: "Play Week 3" })).toBeEnabled();
      await seen.context.close();
    });
  }

  test("a link that closes while someone is on the page turns the page into the closed notice", async ({ page, browser, playwright }) => {
    const seen = await readyPage(page, browser, playwright);
    await seen.page.route(`**/v1/s/${seen.code}/play`, (route) => route.fulfill({ status: 410, json: { error: { code: "link_unavailable", message: "m" } } }));
    await seen.page.getByRole("button", { name: "Play Week 3" }).click();
    await expect(seen.page.getByRole("heading", { level: 1 })).toContainText("no longer available");
    await seen.context.close();
  });
});

test.describe("accessibility", () => {
  for (const [name, size] of [
    ["desktop", { width: 1280, height: 800 }],
    ["phone", { width: 390, height: 844 }],
  ] as const) {
    test(`the share page passes axe at ${name} size, in each state`, async ({ page, browser, playwright }) => {
      test.setTimeout(90_000);
      await owner(page, playwright);
      const closed = await shared(page, "Closed one");
      await revokeLink(page, closed.recording.id, closed.id);
      const waiting = await shared(page, "Week 3", { video: "processing" });
      const locked = await shared(page, "Locked", { password: "open sesame" });
      const idle = await shared(page, "Week 3");

      const { context, page: seen } = await visitor(browser);
      await seen.setViewportSize(size);
      const axe = async (where: string) => {
        await seen.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
        const result = await seen.evaluate(() =>
          // @ts-expect-error axe is added to the page by the script above
          window.axe.run(document, { runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"] } }),
        );
        const summary = result.violations.map((v: { id: string; help: string; nodes: { target: string[] }[] }) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).join(", ")})`);
        expect(summary, where).toEqual([]);
        const overflow = await seen.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
        expect(overflow, `${where} overflows`).toBeLessThanOrEqual(0);
      };

      await seen.goto(`/s/${"A".repeat(22)}`);
      await axe(`not found at ${name} size`);
      await seen.goto(`/s/${closed.code}`);
      await axe(`closed at ${name} size`);
      await seen.goto(`/s/${idle.code}`);
      await axe(`unfinished recording at ${name} size`);
      await seen.goto(`/s/${waiting.code}`);
      await axe(`preparing at ${name} size`);
      await seen.goto(`/s/${locked.code}`);
      await axe(`protected at ${name} size`);

      // The cover, as a ready video shows it.
      await seen.route(`**/v1/s/${waiting.code}`, (route) => route.fulfill({ json: SHARE_JSON(waiting.code) }));
      await seen.goto(`/s/${waiting.code}`);
      await expect(seen.getByRole("button", { name: "Play Week 3" })).toBeVisible({ timeout: 20_000 });
      await axe(`the player cover at ${name} size`);
      await context.close();
    });
  }
});

// Does a real encrypted video play through the share page, in real Chrome?
//
// The browser tests cannot answer that: Playwright's Chromium cannot decode H.264. This uses the
// real thing end to end: a video made with ffmpeg is uploaded through the API, encoded by the real
// worker, protected by the real key server, and played from a share link in an installed Chrome.
// Then the owner revokes the link while it plays, and the script reports what that did.
//
//   node scripts/real-playback.mjs --api-log <file>
//
// Needs running: Postgres, Redis, MinIO (docker compose), the API with `LOG_FORMAT=json` writing to
// <file> (the development mail transport prints the confirmation link there), the worker, the key
// server, and the web app (`npm run start`), plus ffmpeg and Chrome on this machine. Exits 0 if the
// video played. What the revoke did is reported, not judged: see docs/web.md.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { chromium, request as pwRequest } from "@playwright/test";

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? args[at + 1] : fallback;
};
const apiLog = option("api-log");
if (!apiLog) {
  console.error("Usage: node scripts/real-playback.mjs --api-log <the API's JSON log> [--base http://localhost:3000] [--seconds 60]");
  process.exit(2);
}
const BASE = option("base", "http://localhost:3000");
const SECONDS = Number(option("seconds", "60"));
// `--keys-only` shuts the player's own check of its link out, so what ends a revoked viewing is the key server
// alone (the check is the second line of defence, and would hide how far the keys reach). `--wait` lets the
// video play that many seconds before the revoke, to see where in a key period it lands.
const KEYS_ONLY = args.includes("--keys-only");
const WAIT = Number(option("wait", "0"));
const out = fs.mkdtempSync(path.join(os.tmpdir(), "zeno-playback-"));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const started = Date.now();
const say = (...parts) => console.log(`${((Date.now() - started) / 1000).toFixed(1).padStart(6)}s`, ...parts);

function mailedToken(email) {
  for (const line of fs.readFileSync(apiLog, "utf8").split("\n")) {
    if (!line.startsWith("{")) continue;
    try {
      const entry = JSON.parse(line);
      if (entry.fields?.to !== email) continue;
      const match = /\/verify-email\/([A-Za-z0-9_-]+)/.exec(entry.fields.message ?? "");
      if (match) return match[1];
    } catch {
      /* not a log line we care about */
    }
  }
  return null;
}

// A video with a moving picture and a tone.
const video = path.join(out, "check.mp4");
const made = spawnSync("ffmpeg", ["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100", "-t", String(SECONDS), "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "veryfast", "-g", "60", "-c:a", "aac", "-shortest", video]);
if (made.status !== 0) {
  console.error("ffmpeg could not make the test video:", String(made.stderr));
  process.exit(2);
}
const file = fs.readFileSync(video);

const api = await pwRequest.newContext({ baseURL: BASE });
const email = `playback.${Date.now().toString(36)}@example.com`;
await api.post("/v1/auth/register", { data: { name: "Playback Check", email, password: "correct horse battery staple", role: "creator" } });
let token = null;
for (let tries = 0; tries < 80 && !token; tries += 1) {
  token = mailedToken(email);
  if (!token) await sleep(250);
}
if (!token) throw new Error("no confirmation email in the API log: is it running with LOG_FORMAT=json and writing to --api-log?");
await api.post("/v1/auth/verify-email", { data: { token } });

const created = await (await api.post("/v1/videos", { data: { filename: "check.mp4", size_bytes: file.length, content_type: "video/mp4" } })).json();
// Parts go straight to storage, each exactly the size its address was signed for.
let offset = 0;
for (const part of created.upload.parts) {
  await api.put(part.url, { data: file.subarray(offset, offset + part.size) });
  offset += part.size;
}
await api.post(`/v1/videos/${created.video.id}/complete`);
const uploadedAt = Date.now();
let status = "";
for (let tries = 0; tries < 240 && status !== "ready" && status !== "failed"; tries += 1) {
  status = (await (await api.get(`/v1/videos/${created.video.id}`)).json()).video.status;
  if (status !== "ready" && status !== "failed") await sleep(1000);
}
if (status !== "ready") throw new Error(`the worker left the video ${status}`);
say(`video ready ${((Date.now() - uploadedAt) / 1000).toFixed(1)}s after upload (a ${SECONDS} s clip, ${(file.length / 1e6).toFixed(1)} MB)`);

const recording = (await (await api.post("/v1/recordings", { data: { source: "upload", title: "Playback check", video_id: created.video.id } })).json()).recording;
const link = (await (await api.post(`/v1/recordings/${recording.id}/links`, { data: {} })).json()).link;

const browser = await chromium.launch({ channel: "chrome", headless: true });
const page = await (await browser.newContext({ baseURL: BASE, viewport: { width: 1280, height: 800 } })).newPage();
if (KEYS_ONLY) {
  // Only the browser's own asks (the check is a fetch from the page); the page itself was already served.
  await page.route(/\/v1\/s\/[A-Za-z0-9_-]+$/, (route) => (route.request().resourceType() === "fetch" || route.request().resourceType() === "xhr" ? route.abort() : route.continue()));
}
const requests = [];
let playlist = null;
page.on("response", async (response) => {
  const url = response.url();
  if (response.request().method() === "POST" && url.endsWith("/play")) playlist = (await response.json().catch(() => ({}))).playback?.playlist_url ?? null;
  const kind = url.includes(":8081") ? "key" : url.includes(":9000") ? "segment" : url.includes("/v1/play/") ? "playlist" : null;
  if (kind) requests.push({ at: Date.now(), kind, status: response.status() });
});

await page.goto(`/s/${link.code}`);
const pressedAt = Date.now();
await page.getByRole("button", { name: /^Play/ }).click();
try {
  await page.waitForFunction(() => {
    const element = document.querySelector("video");
    return element && element.currentTime > 2 && !element.paused;
  }, null, { timeout: 30_000 });
} catch {
  await page.screenshot({ path: path.join(out, "stuck.png") });
  say("DID NOT PLAY. Screenshot:", path.join(out, "stuck.png"), "Page says:", JSON.stringify(await page.evaluate(() => [...document.querySelectorAll('[role="alert"]')].map((a) => a.textContent))));
  await browser.close();
  process.exit(1);
}
const playing = await page.evaluate(() => {
  const element = document.querySelector("video");
  return { size: `${element.videoWidth}x${element.videoHeight}`, at: element.currentTime, length: element.duration, buffered: element.buffered.length ? element.buffered.end(element.buffered.length - 1) : 0 };
});
say(`PLAYING ${((Date.now() - pressedAt) / 1000).toFixed(1)}s after pressing play: picture ${playing.size}, ${playing.buffered.toFixed(0)} of ${playing.length.toFixed(0)} s already buffered`);
await sleep(2000);
await page.screenshot({ path: path.join(out, "playing.png") });
say("watermark on screen:", await page.getByText(/^Shared view/).isVisible());

// What people say, beside a video that really plays: a reaction lands at the moment the video is at, a comment
// is pinned to it, and pressing a time goes back there. (The browser tests cannot decode H.264, so this is the only
// place the comments meet a real player.)
const signedIn = await page.request.post("/v1/auth/login", { data: { email, password: "correct horse battery staple" } });
say("signed in to leave comments:", signedIn.status());
const seconds = (label) => {
  const parts = label.trim().split(":").map(Number);
  return parts.reduce((total, part) => total * 60 + part, 0);
};
const position = () => page.evaluate(() => document.querySelector("video").currentTime);
let commentsOk = true;
const check = (what, ok, detail) => {
  say(ok ? "ok  " : "FAIL", what, detail ?? "");
  if (!ok) commentsOk = false;
};

const reactedAt = await position();
await page.getByRole("button", { name: "React with 🔥" }).click();
const reaction = page.getByRole("listitem").filter({ hasText: "🔥" }).first();
await reaction.waitFor({ timeout: 10_000 });
const reactionTime = seconds(await reaction.getByRole("button", { name: /^Go to/ }).innerText());
check("a reaction is pinned to the moment the video is at", Math.abs(reactionTime - reactedAt) <= 3, `(video at ${reactedAt.toFixed(1)} s, pinned at ${reactionTime} s)`);

await page.getByLabel("Add a comment").fill("This part, right here");
const commentedAt = await position();
await page.getByRole("button", { name: /^Comment/ }).click();
const comment = page.getByRole("listitem").filter({ hasText: "This part, right here" });
await comment.waitFor({ timeout: 10_000 });
const commentTime = seconds(await comment.getByRole("button", { name: /^Go to/ }).innerText());
check("a comment is pinned to the moment it was written at", Math.abs(commentTime - commentedAt) <= 3, `(video at ${commentedAt.toFixed(1)} s, pinned at ${commentTime} s)`);

await sleep(3000);
const wandered = await position();
await comment.getByRole("button", { name: /^Go to/ }).click();
await sleep(800);
const back = await position();
check("pressing a time goes back to it", wandered - back > 1 && Math.abs(back - commentTime) <= 2, `(was at ${wandered.toFixed(1)} s, now ${back.toFixed(1)} s, asked for ${commentTime} s)`);
check("the video is still playing after that", !(await page.evaluate(() => document.querySelector("video").paused)));
await page.screenshot({ path: path.join(out, "comments.png"), fullPage: true });
if (!commentsOk) {
  await page.screenshot({ path: path.join(out, "comments-failed.png") });
  await browser.close();
  process.exit(1);
}

if (WAIT > 0) await sleep(WAIT * 1000);
// The owner revokes the link while the visitor watches.
const atRevoke = await page.evaluate(() => document.querySelector("video").currentTime);
const revoked = await api.delete(`/v1/recordings/${recording.id}/links/${link.id}`);
const revokedAt = Date.now();
say("REVOKED, the API answered", revoked.status(), `(the video was at ${atRevoke.toFixed(1)} s${KEYS_ONLY ? ", the player's own link check is shut out" : ""})`);
say("the old playlist address now answers", playlist ? (await api.get(playlist)).status() : "n/a", "(a new request is refused at once)");
let ending = "kept playing";
for (let tries = 0; tries < Math.ceil(SECONDS * 2 + 20); tries += 1) {
  const state = await page.evaluate(() => {
    const element = document.querySelector("video");
    return { video: !!element, over: !!element?.ended, notice: [...document.querySelectorAll('[role="alert"]')].map((a) => a.textContent).join(" ") };
  });
  if (!state.video || /ended|stopped/i.test(state.notice)) {
    ending = `the viewing ended ${((Date.now() - revokedAt) / 1000).toFixed(1)} s after the revoke`;
    break;
  }
  if (state.over) {
    ending = `played to its natural end ${((Date.now() - revokedAt) / 1000).toFixed(1)} s after the revoke`;
    break;
  }
  await sleep(500);
}
const after = requests.filter((r) => r.at > revokedAt);
const tally = {};
for (const r of after) tally[`${r.kind} ${r.status}`] = (tally[`${r.kind} ${r.status}`] ?? 0) + 1;
say("after the revoke:", ending);
const endedAt = await page.evaluate(() => document.querySelector("video")?.currentTime ?? null);
say("the video had reached", endedAt === null ? "n/a (removed)" : `${endedAt.toFixed(1)} s`, "when it ended");
say("requests after the revoke, by kind and answer:", JSON.stringify(tally));
console.log("\nScreenshots and the test video are in", out);
await browser.close();

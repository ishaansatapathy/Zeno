// How long after a recording stops can somebody open its link and play it?
//
// This is the measurement behind "link plays within seconds of stop". It does what the extension does (a
// recording of a given length, uploaded in parts, then completed) and then watches the real API and the real
// worker: when the video first becomes playable, when its whole ladder is there, and whether a viewer with
// the link can really fetch the playlists and the first segment at that point.
//
//   node scripts/first-play.mjs --api-log <file> [--api http://127.0.0.1:8080] [--seconds 60] [--runs 3]
//
// Needs running: Postgres, Redis, MinIO (docker compose), the API with `LOG_FORMAT=json` writing to <file> (the
// development mail transport prints the confirmation link there), and the worker. Needs ffmpeg. The numbers
// are printed, not judged: they depend on this machine.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? args[at + 1] : fallback;
};
const apiLog = option("api-log");
if (!apiLog) {
  console.error("Usage: node scripts/first-play.mjs --api-log <the API's JSON log> [--api http://127.0.0.1:8080] [--seconds 60] [--runs 3] [--expect 6]");
  process.exit(2);
}
const API = option("api", "http://127.0.0.1:8080");
const SECONDS = Number(option("seconds", "60"));
const RUNS = Number(option("runs", "3"));
// A 720p source with sound: 144p 240p 360p 480p 720p and audio only.
const EXPECT = Number(option("expect", "6"));
const PART = 5 * 1024 * 1024;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** A tiny cookie jar: the API's session is two cookies. */
class Session {
  jar = new Map();
  async call(method, route, body, headers = {}) {
    const cookie = [...this.jar].map(([k, v]) => `${k}=${v}`).join("; ");
    const reply = await fetch(`${API}${route}`, {
      method,
      headers: { ...(body === undefined ? {} : { "content-type": "application/json" }), ...(cookie ? { cookie } : {}), ...headers },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    for (const line of reply.headers.getSetCookie()) {
      const [pair] = line.split(";");
      const at = pair.indexOf("=");
      this.jar.set(pair.slice(0, at), pair.slice(at + 1));
    }
    const text = await reply.text();
    let json = null;
    try {
      json = JSON.parse(text);
    } catch {
      /* not JSON: a playlist or a segment */
    }
    return { status: reply.status, json, text, headers: reply.headers };
  }
}

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

async function account() {
  const session = new Session();
  const email = `first.play.${Date.now().toString(36)}@example.com`;
  const password = "correct horse battery staple";
  const registered = await session.call("POST", "/v1/auth/register", { name: "Asha Rao", email, password, role: "creator" });
  if (registered.status !== 202) throw new Error(`register: ${registered.status} ${registered.text}`);
  let token = null;
  for (let i = 0; i < 100 && !token; i += 1) {
    token = mailedToken(email);
    if (!token) await sleep(100);
  }
  if (!token) throw new Error("no verification email in the API log");
  const verified = await session.call("POST", "/v1/auth/verify-email", { token });
  if (verified.status !== 200) throw new Error(`verify: ${verified.status} ${verified.text}`);
  const login = await session.call("POST", "/v1/auth/login", { email, password });
  if (login.status !== 200) throw new Error(`login: ${login.status} ${login.text}`);
  return session;
}

/** A recording the way the browser makes one: VP9 and Opus in WebM, a moving picture and a tone. */
function makeRecording(dir, seconds) {
  const file = path.join(dir, "recording.webm");
  const made = spawnSync(
    "ffmpeg",
    ["-y", "-loglevel", "error", "-f", "lavfi", "-i", `testsrc2=size=1280x720:rate=30`, "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", String(seconds), "-c:v", "libvpx-vp9", "-b:v", "2500k", "-deadline", "realtime", "-cpu-used", "8", "-row-mt", "1", "-c:a", "libopus", "-shortest", file],
    { encoding: "utf8" },
  );
  if (made.status !== 0) throw new Error(`ffmpeg could not make the recording: ${made.stderr}`);
  return file;
}

async function once(session, file) {
  const bytes = fs.readFileSync(file);
  const started = await session.call("POST", "/v1/videos/streaming", { filename: "Zeno recording.webm", content_type: "video/webm", part_size_bytes: PART });
  if (started.status !== 201) throw new Error(`start: ${started.status} ${started.text}`);
  const videoId = started.json.video.id;
  const recordingId = crypto.randomUUID();
  const made = await session.call("POST", "/v1/recordings", { id: recordingId, title: "First play", source: "extension", video_id: videoId });
  if (made.status !== 201) throw new Error(`recording: ${made.status} ${made.text}`);
  const link = await session.call("POST", `/v1/recordings/${recordingId}/links`, {});
  if (link.status !== 201) throw new Error(`link: ${link.status} ${link.text}`);
  const code = link.json.link.code;

  // The recording is "made": send it in parts (the extension does this while recording).
  for (let start = 0, number = 1; start < bytes.length; start += PART, number += 1) {
    const end = Math.min(start + PART, bytes.length);
    const part = await session.call("POST", `/v1/videos/${videoId}/parts`, { number, size: end - start, last: end === bytes.length });
    if (part.status !== 200) throw new Error(`part ${number}: ${part.status} ${part.text}`);
    const put = await fetch(part.json.part.url, { method: "PUT", body: bytes.subarray(start, end) });
    if (!put.ok) throw new Error(`storage refused part ${number}: ${put.status}`);
  }

  // The person stops. This is the moment everything is measured from.
  const stopped = Date.now();
  const done = await session.call("POST", `/v1/videos/${videoId}/complete`, { size_bytes: bytes.length });
  if (done.status !== 200) throw new Error(`complete: ${done.status} ${done.text}`);
  const completed = Date.now() - stopped;

  // Somebody opens the link as soon as it plays.
  const viewer = new Session();
  let playable = null;
  let ladder = null;
  for (;;) {
    if (Date.now() - stopped > 10 * 60_000) throw new Error("gave up waiting");
    const video = await session.call("GET", `/v1/videos/${videoId}`);
    if (video.json?.video?.status === "failed") throw new Error("the video failed");
    const count = Array.isArray(video.json?.video?.renditions) ? video.json.video.renditions.length : 0;
    if (video.json?.video?.status === "ready" && !playable) {
      // The link says it plays: do what a player does (the playlists, then the first segment).
      const share = await viewer.call("GET", `/v1/s/${code}`);
      const play = await viewer.call("POST", `/v1/s/${code}/play`, {});
      if (share.json?.share?.status !== "ready" || play.status !== 201) throw new Error(`the link is not playable: ${share.text} ${play.status} ${play.text}`);
      const url = new URL(play.json.playback.playlist_url, API);
      const master = await viewer.call("GET", `${url.pathname}${url.search}`);
      const variants = (master.text.match(/^#EXT-X-STREAM-INF/gm) ?? []).length;
      const lines = (text) => text.split(/\r?\n/);
      const variantUri = lines(master.text).find((line) => line && !line.startsWith("#"));
      const mediaUrl = new URL(variantUri, url);
      const media = await viewer.call("GET", `${mediaUrl.pathname}${mediaUrl.search}`);
      const segment = lines(media.text).find((line) => line.startsWith("http"));
      const first = segment ? await fetch(segment) : null;
      playable = { ms: Date.now() - stopped, variants, firstSegment: first?.status ?? null, bytes: first ? (await first.arrayBuffer()).byteLength : 0 };
    }
    if (count > 0 && count >= EXPECT) {
      ladder = { ms: Date.now() - stopped, count };
      break;
    }
    await sleep(100);
  }
  return { bytes: bytes.length, completed, playable, ladder };
}

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zeno-first-play-"));
const file = makeRecording(dir, SECONDS);
console.log(`recording: ${SECONDS} s, ${(fs.statSync(file).size / 1024 / 1024).toFixed(1)} MiB (VP9 + Opus, 1280x720)`);
const session = await account();
const rows = [];
for (let run = 1; run <= RUNS; run += 1) {
  const result = await once(session, file);
  rows.push(result);
  console.log(
    `run ${run}: complete answered in ${result.completed} ms; playable after ${(result.playable.ms / 1000).toFixed(1)} s ` +
      `(${result.playable.variants} rendition${result.playable.variants === 1 ? "" : "s"} in the master, first segment ${result.playable.firstSegment} ${result.playable.bytes} B); ` +
      `whole ladder (${result.ladder.count}) after ${(result.ladder.ms / 1000).toFixed(1)} s`,
  );
}
const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
console.log(`median: playable ${(median(rows.map((r) => r.playable.ms)) / 1000).toFixed(1)} s, whole ladder ${(median(rows.map((r) => r.ladder.ms)) / 1000).toFixed(1)} s`);
fs.rmSync(dir, { recursive: true, force: true });

import { allowDevices, badge, copyOut, expect, openPopup, probe, stored, test } from "./fixtures";
import { API } from "./zeno";

// Pausing, the count before a recording, and finishing a recording that was cut short. The camera is the
// browser's fake one, so what is recorded is a moving picture and a tone.

async function cameraOnly(popup: Awaited<ReturnType<typeof openPopup>>) {
  await popup.locator("#screen").uncheck({ force: true });
  await popup.locator("#camera").check({ force: true });
}

test("a paused recording leaves the pause out: the clock stands still, and the file is as long as it was recorded", async ({ context, extensionId, worker }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await cameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();
  await expect.poll(() => badge(worker)).toBe("REC");
  await popup.waitForTimeout(2500);

  await popup.getByRole("button", { name: "Pause" }).click();
  await expect(popup.getByRole("button", { name: "Resume" })).toBeVisible();
  await expect(popup.locator("#recording-label")).toContainText("Paused");
  await expect.poll(() => badge(worker)).toBe("II");
  // The clock does not move while paused.
  const frozen = await popup.locator("#clock").innerText();
  await popup.waitForTimeout(3000);
  await expect(popup.locator("#clock")).toHaveText(frozen);

  await popup.getByRole("button", { name: "Resume" }).click();
  await expect(popup.getByRole("button", { name: "Pause" })).toBeVisible();
  await expect.poll(() => badge(worker)).toBe("REC");
  await popup.waitForTimeout(2500);
  await popup.getByRole("button", { name: "Stop and save" }).click();

  // About five seconds were recorded in about eight on the clock: the screen says five, and so does the file.
  await expect(popup.getByRole("heading", { level: 1 })).toContainText(/Saved 0:0[4-6]/);
  const [recording] = await stored(popup);
  expect(recording?.manifest?.status).toBe("complete");
  expect(recording?.manifest?.durationMs ?? 0).toBeGreaterThan(4000);
  expect(recording?.manifest?.durationMs ?? 0).toBeLessThan(6500);
  expect(recording?.manifest?.pauses, "the pause is written down").toHaveLength(1);
  const joint = recording?.manifest?.joints?.[0];
  expect(joint?.ms ?? 0, "how long it lasted").toBeGreaterThan(2500);
  expect(joint?.ms ?? 0).toBeLessThan(4000);
  expect(joint?.at ?? 0, "and where in the file it happened").toBeGreaterThan(1500);
  expect(joint?.at ?? 0).toBeLessThan(3800);

  const info = probe(await copyOut(popup, recording!.id, "camera.webm"));
  expect(info.video, "a video, with no gap in it").not.toBeNull();
  console.log(`paused ${joint?.ms} ms at ${joint?.at} ms into the file; manifest says ${recording?.manifest?.durationMs} ms; the file is ${info.seconds.toFixed(2)} s`);
  expect(info.seconds, "as long as it was recorded, not as long as it took").toBeGreaterThan(3.5);
  expect(info.seconds).toBeLessThan(6.8);
  await expect.poll(() => badge(worker)).toBe("");
});

test("pausing twice, or resuming a recording that is going, changes nothing", async ({ context, extensionId }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await cameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Pause" })).toBeVisible();

  // A resume while recording is ignored.
  await popup.evaluate(() => chrome.runtime.sendMessage({ type: "recorder.resume" }));
  await popup.waitForTimeout(500);
  expect(await popup.evaluate(async () => ((await chrome.storage.session.get("recorder"))["recorder"] as { pausedAt: number | null }).pausedAt)).toBeNull();

  // Two pauses in a row make one.
  await popup.evaluate(() => chrome.runtime.sendMessage({ type: "recorder.pause" }));
  await expect(popup.getByRole("button", { name: "Resume" })).toBeVisible();
  await popup.evaluate(() => chrome.runtime.sendMessage({ type: "recorder.pause" }));
  await popup.waitForTimeout(500);
  await popup.getByRole("button", { name: "Stop and save" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved");
  const [recording] = await stored(popup);
  expect(recording?.manifest?.pauses).toHaveLength(1);
});

test("a recording left paused is stopped and saved, so the camera is not left on by accident", async ({ context, extensionId, worker }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await cameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Pause" })).toBeVisible();
  await popup.waitForTimeout(1500);
  await popup.getByRole("button", { name: "Pause" }).click();
  await expect(popup.getByRole("button", { name: "Resume" })).toBeVisible();

  // The browser tests are built to give up after eight seconds.
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved", { timeout: 15_000 });
  await expect(popup.locator("#saved-note")).toContainText("left paused");
  const [recording] = await stored(popup);
  expect(recording?.manifest?.status).toBe("complete");
  expect(recording?.manifest?.durationMs ?? 0, "the time paused is not recorded").toBeLessThan(4500);
  await expect.poll(() => badge(worker)).toBe("");
});

test("the count: 3, 2, 1 before anything is recorded, and giving up leaves nothing behind", async ({ context, extensionId, worker }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId, { countdown: true });
  await cameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();

  await expect(popup.locator("#count")).toBeVisible();
  await expect.poll(() => badge(worker)).toMatch(/^[123]$/);
  expect((await stored(popup)).length, "nothing is written while counting").toBe(0);

  await popup.getByRole("button", { name: "Cancel" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Ready when you are");
  await expect.poll(() => badge(worker)).toBe("");
  expect((await stored(popup)).length, "and nothing was recorded").toBe(0);
  const camera = await popup.evaluate(async () => (await navigator.mediaDevices.enumerateDevices()).length);
  expect(camera).toBeGreaterThan(0);

  // Counted all the way: it begins, and what is recorded starts after the count.
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.locator("#count")).toBeVisible();
  const countedAt = Date.now();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible({ timeout: 8000 });
  expect(Date.now() - countedAt, "three seconds of count").toBeGreaterThan(2000);
  await expect.poll(() => badge(worker)).toBe("REC");
  await popup.waitForTimeout(2000);
  await popup.getByRole("button", { name: "Stop and save" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText(/Saved 0:0[1-3]/);
});

test("the count is drawn on the page being shared, and Esc on it cancels", async ({ context, extensionId }) => {
  await allowDevices(context, extensionId);
  // A page the extension may draw on (the API's own address, which it has access to): the tab being looked at.
  const page = await context.newPage();
  await page.goto(`${API}/ready`);
  const popup = await openPopup(context, extensionId, { countdown: true });
  await popup.locator("#screen").uncheck({ force: true });
  await popup.locator("#camera").check({ force: true });
  await page.bringToFront();

  await popup.evaluate(() => chrome.runtime.sendMessage({ type: "recorder.start", options: { screen: false, systemAudio: false, microphone: true, camera: true, countdown: true } }));
  const overlay = page.locator("#zeno-countdown");
  await expect(overlay).toBeAttached();
  await expect(overlay.locator(".n")).toHaveText(/^[123]$/);
  await expect(overlay.locator(".hint")).toContainText("Esc");

  await page.keyboard.press("Escape");
  await expect(overlay).toHaveCount(0);
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Ready when you are");
  expect((await stored(popup)).length, "nothing was recorded").toBe(0);

  // Counted to the end, the number is gone from the page before the recording begins.
  await popup.evaluate(() => chrome.runtime.sendMessage({ type: "recorder.start", options: { screen: false, systemAudio: false, microphone: true, camera: true, countdown: true } }));
  await expect(overlay).toBeAttached();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible({ timeout: 8000 });
  await expect(overlay).toHaveCount(0);
  await popup.getByRole("button", { name: "Stop and save" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved");
});

test("a recording cut short is offered on the start screen, and finished into a real video", async ({ context, extensionId, worker }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await cameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();
  await popup.waitForTimeout(3500);
  await worker.evaluate(() => chrome.offscreen.closeDocument());

  // The popup says so, and then offers to finish it.
  await popup.reload();
  await expect(popup.getByRole("alert")).toContainText("interrupted");
  await popup.getByRole("button", { name: "Try again" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Ready when you are");
  const unfinished = popup.getByRole("region", { name: "Unfinished recordings" });
  await expect(unfinished).toContainText("cut short");

  const [before] = await stored(popup);
  expect(before?.manifest?.status).toBe("recording");
  await unfinished.getByRole("button", { name: /^Recover/ }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved", { timeout: 15_000 });
  await expect(popup.locator("#saved-note")).toContainText("cut short");
  await expect(popup.getByRole("list", { name: "Saved files" })).toContainText("camera.webm");

  // Closed off: it says how long it ran, and it is a video.
  const [after] = await stored(popup);
  expect(after?.manifest?.status).toBe("complete");
  expect(after?.manifest?.recovered).toBe(true);
  expect(after?.manifest?.durationMs ?? 0).toBeGreaterThan(2000);
  const info = probe(await copyOut(popup, after!.id, "camera.webm"));
  expect(info.video).not.toBeNull();
  expect(info.audio?.codec).toBe("opus");
  // Nobody was signed in, so there is no link and nothing was sent anywhere.
  await expect(popup.locator("#share-url")).toHaveCount(0);
  // Handed to the browser to save, as a recording that was stopped would be.
  expect(await worker.evaluate(() => chrome.downloads.search({}))).toHaveLength(1);

  // It is done with: it is not offered again.
  await popup.getByRole("button", { name: "Record again" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Ready when you are");
  await expect(popup.getByRole("region", { name: "Unfinished recordings" })).toHaveCount(0);
});

test("a missing download keeps the old recording and offers to save it again", async ({ context, extensionId, worker }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await cameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();
  await expect.poll(async () => (await stored(popup))[0]?.files.some((file) => file.size > 0)).toBe(true);
  await popup.getByRole("button", { name: "Stop and save" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved");
  await expect.poll(() => worker.evaluate(async () => (await chrome.downloads.search({}))[0]?.state)).toBe("complete");
  await worker.evaluate(async () => {
    const [item] = await chrome.downloads.search({});
    if (!item) throw new Error("Expected a download");
    await chrome.downloads.removeFile(item.id);
  });
  const [recording] = await stored(popup);
  await popup.evaluate(async (id) => {
    const root = await navigator.storage.getDirectory();
    const dir = await (await root.getDirectoryHandle("recordings")).getDirectoryHandle(id);
    const handle = await dir.getFileHandle("manifest.json");
    const manifest = JSON.parse(await (await handle.getFile()).text());
    const writable = await handle.createWritable();
    await writable.write(JSON.stringify({ ...manifest, endedAt: Date.now() - 8 * 24 * 3600 * 1000 }));
    await writable.close();
  }, recording!.id);

  await popup.getByRole("button", { name: "Record again" }).click();
  const unfinished = popup.getByRole("region", { name: "Unfinished recordings" });
  await expect(unfinished).toContainText("local save incomplete");
  expect((await stored(popup))[0]?.id).toBe(recording!.id);
  await unfinished.getByRole("button", { name: /^Save a local copy/ }).click();
  await expect.poll(() => worker.evaluate(async () => (await chrome.downloads.search({})).filter((item) => item.exists && item.state === "complete").length)).toBe(1);
});

test("a recording cut short can be deleted, after asking twice", async ({ context, extensionId, worker }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await cameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();
  await popup.waitForTimeout(2500);
  await worker.evaluate(() => chrome.offscreen.closeDocument());
  await popup.reload();
  await popup.getByRole("button", { name: "Try again" }).click();

  const unfinished = popup.getByRole("region", { name: "Unfinished recordings" });
  await expect(unfinished).toBeVisible();
  const remove = unfinished.getByRole("button", { name: /^Delete the recording/ });
  await remove.click();
  const sure = unfinished.getByRole("button", { name: /^Press again to delete/ });
  await expect(sure).toContainText("for good");
  expect((await stored(popup)).length, "one press deletes nothing").toBe(1);
  await sure.click();
  await expect(popup.getByRole("region", { name: "Unfinished recordings" })).toHaveCount(0);
  expect((await stored(popup)).length).toBe(0);
});

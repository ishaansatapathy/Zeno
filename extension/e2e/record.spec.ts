import { allowDevices, copyOut, expect, openPopup, probe, stored, test } from "./fixtures";

test("the popup offers what can be recorded, and will not start with nothing chosen", async ({ context, extensionId }) => {
  const popup = await openPopup(context, extensionId);
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Ready when you are");
  await expect(popup.getByRole("button", { name: "Start recording" })).toBeEnabled();

  await popup.getByLabel("Screen", { exact: false }).first().uncheck({ force: true });
  await expect(popup.getByRole("button", { name: "Start recording" })).toBeDisabled();
  // The sound of the screen is only offered with a screen.
  await expect(popup.locator("#systemAudio")).toBeDisabled();

  await popup.getByLabel("Camera", { exact: false }).first().check({ force: true });
  await expect(popup.getByRole("button", { name: "Start recording" })).toBeEnabled();
});

test("records the camera and microphone to disk, and what is saved is real video", async ({ context, extensionId, worker }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await popup.locator("#screen").uncheck({ force: true });
  await popup.locator("#camera").check({ force: true });
  await popup.getByRole("button", { name: "Start recording" }).click();

  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();
  await expect(popup.locator("#clock")).not.toHaveText("0:00", { timeout: 5_000 });
  await popup.waitForTimeout(2500);
  await popup.getByRole("button", { name: "Stop and save" }).click();

  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved");
  await expect(popup.getByRole("list", { name: "Saved files" }).getByRole("listitem")).toHaveCount(1);
  await expect(popup.getByRole("list", { name: "Saved files" })).toContainText("camera.webm");

  // On disk, finished, and describing itself.
  const [recording] = await stored(popup);
  expect(recording?.manifest?.status).toBe("complete");
  expect(recording?.manifest?.tracks.map((t) => t.track)).toEqual(["camera"]);
  const camera = recording?.files.find((f) => f.name === "camera.webm");
  expect(camera?.size ?? 0).toBeGreaterThan(10_000);

  // A real file: ffprobe reads a picture and a voice, a few seconds long.
  const info = probe(await copyOut(popup, recording!.id, "camera.webm"));
  expect(info.video?.codec).toMatch(/vp9|vp8/);
  expect(info.video?.width).toBeGreaterThan(0);
  expect(info.audio?.codec).toBe("opus");
  expect(info.seconds).toBeGreaterThan(2);
  expect(info.seconds).toBeLessThan(8);

  // And handed to the browser to save in the person's Downloads folder: one file, from the extension's own
  // storage, and the browser finished saving it. (A test browser keeps downloads under its own names,
  // so the name the extension asked for cannot be read back here.)
  const downloads = await worker.evaluate(() => chrome.downloads.search({}));
  expect(downloads).toHaveLength(1);
  expect(downloads[0]?.url).toMatch(/^blob:chrome-extension:\/\//);
  expect(downloads[0]?.state).toBe("complete");
});

test("a recording that is killed in the middle keeps what had been written", async ({ context, extensionId, worker }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await popup.locator("#screen").uncheck({ force: true });
  await popup.locator("#camera").check({ force: true });
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();
  await popup.waitForTimeout(3500);

  // The recorder is taken away without being asked to stop: a crash, as far as the recording can tell.
  await worker.evaluate(() => chrome.offscreen.closeDocument());

  const [recording] = await stored(popup);
  expect(recording?.manifest?.status, "it never got to say it was finished").toBe("recording");
  const camera = recording?.files.find((f) => f.name === "camera.webm");
  expect(camera?.size ?? 0, "but the chunks written before the crash are on disk").toBeGreaterThan(10_000);
  const info = probe(await copyOut(popup, recording!.id, "camera.webm"));
  expect(info.video, "and they are a video").not.toBeNull();

  // The next time anyone looks, the popup says so instead of showing a recorder that is not there.
  await popup.reload();
  await expect(popup.getByRole("alert")).toContainText("interrupted");
  await popup.getByRole("button", { name: "Try again" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Ready when you are");
});

test("closing the popup does not lose the recording, and reopening it shows the time", async ({ context, extensionId }) => {
  await allowDevices(context, extensionId);
  const first = await openPopup(context, extensionId);
  await first.locator("#screen").uncheck({ force: true });
  await first.locator("#camera").check({ force: true });
  await first.getByRole("button", { name: "Start recording" }).click();
  await expect(first.getByRole("button", { name: "Stop and save" })).toBeVisible();
  await first.close();

  await new Promise((resolve) => setTimeout(resolve, 2500));
  const second = await openPopup(context, extensionId);
  await expect(second.getByRole("button", { name: "Stop and save" })).toBeVisible();
  await expect(second.locator("#clock")).not.toHaveText("0:00");
  await second.getByRole("button", { name: "Stop and save" }).click();
  await expect(second.getByRole("heading", { level: 1 })).toContainText("Saved");
});

test("a second start while recording is refused, not doubled", async ({ context, extensionId }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await popup.locator("#screen").uncheck({ force: true });
  await popup.locator("#camera").check({ force: true });
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();

  const again = await popup.evaluate(() => chrome.runtime.sendMessage({ type: "recorder.start", options: { screen: false, systemAudio: false, microphone: true, camera: true } }));
  expect((again as { phase: string }).phase).toBe("recording");
  expect((await stored(popup)).length).toBe(1);
  await popup.getByRole("button", { name: "Stop and save" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved");
});

test("the camera bubble shows the camera", async ({ context, extensionId }) => {
  await allowDevices(context, extensionId);
  const page = await context.newPage();
  await page.goto(`chrome-extension://${extensionId}/bubble.html`);
  await expect.poll(() => page.evaluate(() => (document.getElementById("camera") as HTMLVideoElement).videoWidth)).toBeGreaterThan(0);
});

import { allowDevices, expect, openPopup, stored, test } from "./fixtures";
import { ask, signedInAs } from "./zeno";

// Finishing a recording whose upload was cut off, against the real API and object storage.

async function cameraOnly(popup: Awaited<ReturnType<typeof openPopup>>) {
  await popup.locator("#screen").uncheck({ force: true });
  await popup.locator("#camera").check({ force: true });
}

test("a recording cut short after part of it went up is finished under the same link, and Zeno ends up with all of it", async ({ context, extensionId, worker }) => {
  test.setTimeout(240_000);
  await signedInAs(context);
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await cameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();
  const field = popup.locator("#share-url");
  await expect(field).toBeVisible({ timeout: 15_000 });
  const link = await field.inputValue();

  // Part of it is in Zeno (a part is 5 MiB) when the recorder is taken away without being asked to stop.
  await expect(popup.locator("#upload-line")).toContainText(/Uploading as you record · [1-9]/, { timeout: 90_000 });
  const [recording] = await stored(popup);
  const id = recording?.id ?? "";
  const held = await ask<{ uploaded: number[] }>(context, `/v1/videos/${id}/upload`);
  expect(held.body.uploaded, "storage holds the first part").toContain(1);
  expect(recording?.upload?.status, "the extension knows the upload is not done").toBe("pending");
  await worker.evaluate(() => chrome.offscreen.closeDocument());

  await popup.reload();
  await expect(popup.getByRole("alert")).toContainText("interrupted");
  await popup.getByRole("button", { name: "Try again" }).click();
  const unfinished = popup.getByRole("region", { name: "Unfinished recordings" });
  await expect(unfinished).toContainText("cut short");
  await unfinished.getByRole("button", { name: /^Recover/ }).click();

  // Saved as a recording that was stopped, with the link that was made when it started.
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved", { timeout: 20_000 });
  expect(await popup.locator("#share-url").inputValue(), "the same link, not a new one").toBe(link);
  await expect(popup.locator("#upload-line")).toContainText("Uploaded", { timeout: 90_000 });

  // Zeno holds exactly what is on this computer, and nothing was made twice.
  const [finished] = await stored(popup);
  expect(finished?.upload?.status).toBe("done");
  expect(finished?.manifest?.status).toBe("complete");
  const local = finished?.files.find((file) => file.name === "camera.webm");
  expect(local?.size ?? 0).toBeGreaterThan(5 * 1024 * 1024);
  const mine = await ask<{ recording: { id: string; video_id: string; status: string } }>(context, `/v1/recordings/${id}`);
  expect(mine.status).toBe(200);
  expect(mine.body.recording.id).toBe(id);
  const video = await ask<{ video: { size_bytes: number; status: string } }>(context, `/v1/videos/${mine.body.recording.video_id}`);
  expect(video.body.video.size_bytes, "all of it, byte for byte").toBe(local?.size);
  expect(video.body.video.status).toBe("processing");
  const list = await ask<{ recordings: { id: string }[] }>(context, "/v1/recordings");
  expect(list.body.recordings.filter((item) => item.id === id), "one recording, not two").toHaveLength(1);
  const code = link.split("/").pop();
  expect((await ask<{ share: { status: string } }>(context, `/v1/s/${code}`)).body.share.status).toBe("processing");
});

test("an upload that did not finish is finished from the start screen, without saving the files again", async ({ context, extensionId, worker }) => {
  test.setTimeout(120_000);
  await signedInAs(context);
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await cameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();
  await expect(popup.locator("#share-url")).toBeVisible({ timeout: 15_000 });
  const link = await popup.locator("#share-url").inputValue();
  await popup.waitForTimeout(2000);
  await popup.getByRole("button", { name: "Stop and save" }).click();
  await expect(popup.locator("#upload-line")).toContainText("Uploaded", { timeout: 60_000 });
  await expect.poll(async () => (await stored(popup))[0]?.upload?.status).toBe("done");
  const [recording] = await stored(popup);
  const id = recording?.id ?? "";
  const bytes = recording?.files.find((file) => file.name === "camera.webm")?.size ?? 0;

  // Say it did not finish (as it would have been had the network gone away at the end).
  await popup.evaluate(async (recordingId) => {
    const root = await navigator.storage.getDirectory();
    const directory = await (await root.getDirectoryHandle("recordings")).getDirectoryHandle(recordingId);
    const writable = await (await directory.getFileHandle("upload.json", { create: true })).createWritable();
    await writable.write(JSON.stringify({ version: 1, status: "failed", partSize: 5 * 1024 * 1024 }));
    await writable.close();
  }, id);

  const session = await context.cookies();
  await context.clearCookies();
  await popup.getByRole("button", { name: "Record again" }).click();
  const unfinished = popup.getByRole("region", { name: "Unfinished recordings" });
  await expect(unfinished).toContainText("not all in Zeno yet");
  await unfinished.getByRole("button", { name: /^Finish the upload/ }).click();
  await expect(popup.locator("#upload-line")).toContainText("Sign in to Zeno");
  await expect.poll(async () => (await stored(popup))[0]?.upload?.status).toBe("failed");
  await context.addCookies(session);
  await popup.getByRole("button", { name: "Record again" }).click();
  await unfinished.getByRole("button", { name: /^Finish the upload/ }).click();

  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved", { timeout: 15_000 });
  expect(await popup.locator("#share-url").inputValue()).toBe(link);
  await expect(popup.locator("#upload-line")).toContainText("Uploaded", { timeout: 60_000 });
  expect((await stored(popup))[0]?.upload?.status).toBe("done");
  // The files were saved to Downloads when it stopped; finishing an upload does not save them again.
  expect(await worker.evaluate(() => chrome.downloads.search({}))).toHaveLength(1);

  const mine = await ask<{ recording: { video_id: string } }>(context, `/v1/recordings/${id}`);
  const video = await ask<{ video: { size_bytes: number } }>(context, `/v1/videos/${mine.body.recording.video_id}`);
  expect(video.body.video.size_bytes).toBe(bytes);
});

test("a failed upload on the saved screen can be tried again from there", async ({ context, extensionId }) => {
  test.setTimeout(120_000);
  await signedInAs(context);
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await cameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.locator("#share-url")).toBeVisible({ timeout: 15_000 });
  await popup.waitForTimeout(1500);
  await popup.getByRole("button", { name: "Stop and save" }).click();
  await expect(popup.locator("#upload-line")).toContainText("Uploaded", { timeout: 60_000 });
  const [recording] = await stored(popup);
  const id = recording?.id ?? "";

  // The saved screen is told the upload failed (what the uploader says when it gives up), and offers another go.
  await popup.evaluate(
    async ([recordingId, view]) => {
      await chrome.runtime.sendMessage({ target: "background", type: "upload", id: recordingId, view });
    },
    [id, { phase: "failed", sent: 1, total: 1, url: null, copied: false, message: "Could not reach Zeno to finish the upload." }] as const,
  );
  await expect(popup.locator("#upload-line")).toContainText("Could not reach Zeno");
  await popup.evaluate(async (recordingId) => {
    const root = await navigator.storage.getDirectory();
    const directory = await (await root.getDirectoryHandle("recordings")).getDirectoryHandle(recordingId);
    const writable = await (await directory.getFileHandle("upload.json", { create: true })).createWritable();
    await writable.write(JSON.stringify({ version: 1, status: "failed", partSize: 5 * 1024 * 1024 }));
    await writable.close();
  }, id);

  await popup.getByRole("button", { name: "Try the upload again" }).click();
  await expect(popup.locator("#upload-line")).toContainText("Uploaded", { timeout: 60_000 });
  expect((await stored(popup))[0]?.upload?.status).toBe("done");
});

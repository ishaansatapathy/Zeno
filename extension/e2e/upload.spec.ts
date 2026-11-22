import { allowDevices, expect, openPopup, stored, test } from "./fixtures";
import { API, ask, signedInAs } from "./zeno";

// Uploading while recording, against the real API and object storage. The camera is the browser's fake one, so
// what is recorded is a moving picture and a tone.

async function chooseCameraOnly(popup: Awaited<ReturnType<typeof openPopup>>) {
  await popup.locator("#screen").uncheck({ force: true });
  await popup.locator("#camera").check({ force: true });
}

test("signed out, the recording still works, and the popup says what a link needs", async ({ context, extensionId }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await expect(popup.locator("#sign-in")).toBeVisible();
  await expect(popup.locator("#account")).toContainText("Sign in to Zeno to get a share link");

  await chooseCameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();
  await expect(popup.locator("#upload-line")).toContainText("sign in to Zeno to get a link");
  await expect(popup.locator("#share-url")).toHaveCount(0);
  await popup.waitForTimeout(1500);
  await popup.getByRole("button", { name: "Stop and save" }).click();

  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved");
  await expect(popup.getByRole("list", { name: "Saved files" })).toContainText("camera.webm");
  const [recording] = await stored(popup);
  expect(recording?.manifest?.status).toBe("complete");
});

test("signed in, the link exists while recording, is on the clipboard at stop, and the recording is all in Zeno", async ({ context, extensionId }) => {
  test.setTimeout(150_000);
  // A page of a real site reads the clipboard (an extension page cannot be granted the permission).
  await context.grantPermissions(["clipboard-read"], { origin: API });
  const { email } = await signedInAs(context);
  await allowDevices(context, extensionId);

  const popup = await openPopup(context, extensionId);
  await expect(popup.locator("#account")).toContainText(email);
  await chooseCameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible();

  // The link is there before the recording ends: the person can paste it into a chat already.
  const field = popup.locator("#share-url");
  await expect(field).toBeVisible({ timeout: 15_000 });
  const link = await field.inputValue();
  expect(link).toMatch(/^http:\/\/localhost:3000\/s\/[A-Za-z0-9_-]{22}$/);

  // Parts go up while it is still recording (a part is 5 MiB), and the number says so.
  await expect(popup.locator("#upload-line")).toContainText(/Uploading as you record · [1-9]/, { timeout: 90_000 });
  const [recording] = await stored(popup);
  expect(recording?.manifest?.status, "still recording").toBe("recording");

  const stopAt = Date.now();
  await popup.getByRole("button", { name: "Stop and save" }).click();
  await expect(popup.locator("#copied")).toBeVisible();
  const stopToLink = Date.now() - stopAt;
  console.log(`stop to link on the clipboard: ${stopToLink} ms`);
  expect(stopToLink).toBeLessThan(1500);
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved");
  const reader = await context.newPage();
  await reader.goto(`${API}/ready`);
  expect(await reader.evaluate(() => navigator.clipboard.readText()), "the link is on the clipboard").toBe(link);
  await reader.close();
  expect(await popup.locator("#share-url").inputValue()).toBe(link);

  // Then the rest goes up and the video is assembled.
  await expect(popup.locator("#upload-line")).toContainText("Uploaded", { timeout: 60_000 });

  const finished = (await stored(popup))[0];
  const local = finished?.files.find((file) => file.name === "camera.webm");
  expect(local?.size ?? 0).toBeGreaterThan(5 * 1024 * 1024);

  const recordingId = finished?.id ?? "";
  const mine = await ask<{ recording: { id: string; video_id: string; source: string; status: string } }>(context, `/v1/recordings/${recordingId}`);
  expect(mine.status).toBe(200);
  expect(mine.body.recording).toMatchObject({ id: recordingId, source: "extension", status: "processing" });
  const video = await ask<{ video: { size_bytes: number; status: string; original_filename: string } }>(context, `/v1/videos/${mine.body.recording.video_id}`);
  expect(video.body.video.status).toBe("processing");
  expect(video.body.video.size_bytes, "Zeno holds exactly the bytes that were recorded").toBe(local?.size);
  expect(video.body.video.original_filename).toMatch(/^Zeno .* camera\.webm$/);

  // The link already opens: it says the recording is being prepared.
  const code = link.split("/").pop();
  const page = await ask<{ share: { status: string } }>(context, `/v1/s/${code}`);
  expect(page.status).toBe(200);
  expect(page.body.share.status).toBe("processing");
});

test("an account that cannot upload (a student's) records locally and is told so", async ({ context, extensionId }) => {
  await signedInAs(context, "student");
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await chooseCameraOnly(popup);
  await popup.getByRole("button", { name: "Start recording" }).click();
  await expect(popup.locator("#upload-line")).toContainText("This account cannot upload recordings", { timeout: 15_000 });
  await popup.waitForTimeout(1000);
  await popup.getByRole("button", { name: "Stop and save" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved");
  await expect(popup.locator("#share-url")).toHaveCount(0);
});

import { allowDevices, copyOut, expect, openPopup, probe, stored, test } from "./fixtures";

// Recording the screen needs a screen, so these run only where there is one: `ZENO_SCREEN=1 npm run e2e`
// opens a visible browser window and lets it choose the screen without showing its picker.
test.skip(!process.env["ZENO_SCREEN"], "needs a display: run with ZENO_SCREEN=1");

test("records the screen and the camera as two files, with the voice in the screen's", async ({ context, extensionId }) => {
  await allowDevices(context, extensionId);
  const popup = await openPopup(context, extensionId);
  await popup.locator("#camera").check({ force: true });
  await popup.getByRole("button", { name: "Start recording" }).click();

  await expect(popup.getByRole("button", { name: "Stop and save" })).toBeVisible({ timeout: 20_000 });
  await popup.waitForTimeout(3000);
  await popup.getByRole("button", { name: "Stop and save" }).click();
  await expect(popup.getByRole("heading", { level: 1 })).toContainText("Saved");

  const [recording] = await stored(popup);
  expect(recording?.manifest?.status).toBe("complete");
  expect(recording?.manifest?.tracks.map((t) => t.track).sort()).toEqual(["camera", "screen"]);

  const screen = probe(await copyOut(popup, recording!.id, "screen.webm"));
  expect(screen.video?.width).toBeGreaterThan(300);
  expect(screen.audio?.codec, "the microphone is in the screen's file").toBe("opus");
  expect(screen.seconds).toBeGreaterThan(2);

  const camera = probe(await copyOut(popup, recording!.id, "camera.webm"));
  expect(camera.video).not.toBeNull();
  expect(camera.audio, "and not in the camera's").toBeNull();
});

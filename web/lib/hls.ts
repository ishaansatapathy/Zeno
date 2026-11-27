// Plays an HLS playlist in a <video>: the one way every viewer in the app does it, so the share page and the team's
// player do not each have a copy that could drift.

export const ENDED = "This viewing has ended. Press play to start again.";

/**
 * hls.js plays it wherever the browser has Media Source Extensions (even where the browser can play HLS itself:
 * Chrome now can, and hls.js is the one whose errors we can read and act on), and is only downloaded now, so a page
 * that nobody plays costs no player code. Only a browser without them (an iPhone before iOS 17.1) uses its own HLS.
 * A viewing that is refused (closed, expired) or fails for good calls `ended` with a sentence to show. Returns what
 * stops it.
 */
export function attachPlaylist(element: HTMLVideoElement, playlistUrl: string, ended: (notice: string) => void): () => void {
  let cancelled = false;
  let hls: { destroy: () => void } | null = null;

  void (async () => {
    const { default: Hls } = await import("hls.js");
    if (cancelled) return;
    if (!Hls.isSupported()) {
      // No Media Source Extensions (an iPhone before iOS 17.1): the browser's own HLS is all there is.
      if (element.canPlayType("application/vnd.apple.mpegurl")) {
        element.addEventListener("error", () => ended("Playback stopped. Press play to try again."));
        element.src = playlistUrl;
        void element.play().catch(() => undefined);
      } else {
        ended("This browser can't play this video. Try a recent Chrome, Edge, Firefox or Safari.");
      }
      return;
    }
    const player = new Hls({
      // hls.js starts at the lowest rendition and assumes a slow connection until it has measured
      // one, which at the speed a clip downloads means the whole thing arrives in 144p. Assume a
      // decent connection to start (it corrects itself after the first segment, and steps down if
      // it was wrong), do not fetch more picture than the player can show, and do not run
      // minutes ahead of the viewer, so a better rendition can still be chosen for what is to come.
      abrEwmaDefaultEstimate: 4_000_000,
      capLevelToPlayerSize: true,
      maxBufferLength: 30,
      maxMaxBufferLength: 60,
    });
    hls = player;
    let recoveries = 0;
    player.on(Hls.Events.ERROR, (_event, data) => {
      if (cancelled) return;
      const status = data.response?.code;
      // The server said no: the link closed, or the viewing ran out. hls.js keeps asking again
      // and reports each refusal as a non-fatal error, so the first one is the answer, fatal or
      // not. Retrying cannot help.
      if (status === 401 || status === 403 || status === 404 || status === 410) return ended(ENDED);
      if (!data.fatal) return;
      if (recoveries < 2 && data.type === Hls.ErrorTypes.NETWORK_ERROR) {
        recoveries += 1;
        return player.startLoad();
      }
      if (recoveries < 2 && data.type === Hls.ErrorTypes.MEDIA_ERROR) {
        recoveries += 1;
        return player.recoverMediaError();
      }
      ended("Playback stopped. Press play to try again.");
    });
    player.on(Hls.Events.MANIFEST_PARSED, () => void element.play().catch(() => undefined));
    player.loadSource(playlistUrl);
    player.attachMedia(element);
  })();

  return () => {
    cancelled = true;
    hls?.destroy();
  };
}

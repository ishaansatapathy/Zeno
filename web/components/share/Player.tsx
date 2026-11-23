"use client";

import { useEffect, useReducer, useRef, useState, type FormEvent, type MutableRefObject } from "react";

import { Notice, PasswordField, Submit, focusField } from "@/components/auth/controls";
import { describeFailure } from "@/components/auth/messages";
import { clock } from "@/components/library/rows";
import { api } from "@/lib/api/client";

import type { PlayerControl } from "./conversation";
import { LINK_CHECK_SECONDS, initialPlayerState, nextPlayerState, preparingText } from "./state";

const ENDED = "This viewing has ended. Press play to start again.";

/** Where the watermark sits, in turn. It moves so that it cannot be cropped out of a recording of the screen. */
const SPOTS = ["left-4 top-4", "right-4 top-4", "right-4 bottom-16", "left-4 bottom-16"];

function Watermark({ text }: { text: string }) {
  const [spot, setSpot] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setSpot((now) => (now + 1) % SPOTS.length), 25_000);
    return () => clearInterval(timer);
  }, []);
  return (
    <div aria-hidden="true" className={`pointer-events-none absolute ${SPOTS[spot]} select-none font-mono text-[0.7rem] tracking-wider text-white/40`}>
      {text}
    </div>
  );
}

/**
 * The video, once a viewing has started. hls.js plays it wherever the browser has Media Source
 * Extensions (even where the browser can play HLS itself: Chrome now can, and hls.js is the one
 * whose errors we can read and act on), and is only downloaded now, so a page that nobody plays costs
 * no player code. Only a browser without them (an iPhone before iOS 17.1) uses its own HLS. A viewing that is
 * refused (closed, expired) or fails for good hands control back with a sentence to show.
 */
function Viewing({ code, playlistUrl, expiresAt, watermark, title, control, onEnded, onClosed }: { code: string; playlistUrl: string; expiresAt: number; watermark: string; title: string; control?: MutableRefObject<PlayerControl | null>; onEnded: (notice: string) => void; onClosed: () => void }) {
  const video = useRef<HTMLVideoElement>(null);
  const ended = useRef(onEnded);
  ended.current = onEnded;
  const closed = useRef(onClosed);
  closed.current = onClosed;

  useEffect(() => {
    const element = video.current;
    if (!element) return;
    let cancelled = false;
    let hls: { destroy: () => void } | null = null;

    void (async () => {
      const { default: Hls } = await import("hls.js");
      if (cancelled) return;
      if (!Hls.isSupported()) {
        // No Media Source Extensions (an iPhone before iOS 17.1): the browser's own HLS is all there is.
        if (element.canPlayType("application/vnd.apple.mpegurl")) {
          element.addEventListener("error", () => ended.current("Playback stopped. Press play to try again."));
          element.src = playlistUrl;
          void element.play().catch(() => undefined);
        } else {
          ended.current("This browser can't play this video. Try a recent Chrome, Edge, Firefox or Safari.");
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
        if (status === 401 || status === 403 || status === 404 || status === 410) return ended.current(ENDED);
        if (!data.fatal) return;
        if (recoveries < 2 && data.type === Hls.ErrorTypes.NETWORK_ERROR) {
          recoveries += 1;
          return player.startLoad();
        }
        if (recoveries < 2 && data.type === Hls.ErrorTypes.MEDIA_ERROR) {
          recoveries += 1;
          return player.recoverMediaError();
        }
        ended.current("Playback stopped. Press play to try again.");
      });
      player.on(Hls.Events.MANIFEST_PARSED, () => void element.play().catch(() => undefined));
      player.loadSource(playlistUrl);
      player.attachMedia(element);
    })();

    return () => {
      cancelled = true;
      hls?.destroy();
    };
  }, [playlistUrl]);

  // What the comments beside the video need of it: where it is, how long it is, and a way to go somewhere.
  useEffect(() => {
    const element = video.current;
    if (!element || !control) return;
    control.current = {
      positionMs: () => element.currentTime * 1000,
      durationMs: () => (Number.isFinite(element.duration) ? element.duration * 1000 : null),
      seek: (ms) => {
        element.currentTime = Math.max(0, ms / 1000);
        void element.play().catch(() => undefined);
      },
    };
    return () => {
      control.current = null;
    };
  }, [control]);

  // Revoking a link stops new viewings and new keys at once, but a player that already holds the key
  // and the playlist would carry on to the end of the video without asking anyone anything. So the
  // viewing asks, now and then, whether its link is still open, and stops when it is not.
  useEffect(() => {
    const timer = setInterval(async () => {
      const result = await api.share.resolve(code);
      if (!result.ok && (result.error.status === 404 || result.error.status === 410)) closed.current();
    }, LINK_CHECK_SECONDS * 1000);
    return () => clearInterval(timer);
  }, [code]);

  // A viewing has a length. When it is up the server stops giving keys, so say so rather than stall.
  useEffect(() => {
    const timer = setTimeout(() => ended.current(ENDED), Math.max(0, expiresAt - Date.now()));
    return () => clearTimeout(timer);
  }, [expiresAt]);

  return (
    <div className="relative bg-black">
      <video ref={video} controls playsInline controlsList="nodownload" disablePictureInPicture aria-label={title} className="aspect-video w-full bg-black" />
      <Watermark text={watermark} />
    </div>
  );
}

/**
 * A share link's video: a cover with a play button (and a password box, when the link has one),
 * then the player. Everything that can happen on the way is a sentence, never a dead end.
 */
export function Player({
  code,
  title,
  durationSeconds,
  needsPassword,
  control,
  onViewing,
  onPreparing,
  onClosed,
}: {
  code: string;
  title: string;
  durationSeconds: number | null;
  needsPassword: boolean;
  /** Filled while a viewing is under way, for whatever sits beside the video and wants to follow or steer it. */
  control?: MutableRefObject<PlayerControl | null>;
  /** The token of the viewing (the `t` of its playlist address) while one is under way, and `null` when it ends. */
  onViewing?: (token: string | null) => void;
  /** The video turned out not to be ready (for a link with no password, the page then waits for it). */
  onPreparing?: () => void;
  onClosed: () => void;
}) {
  const [state, dispatch] = useReducer(nextPlayerState, initialPlayerState);
  const [password, setPassword] = useState("");
  const [empty, setEmpty] = useState(false);

  const token = state.phase === "playing" ? new URL(state.playlistUrl, "http://x").searchParams.get("t") : null;
  useEffect(() => {
    onViewing?.(token);
  }, [token, onViewing]);

  useEffect(() => {
    if (state.phase === "closed") onClosed();
    if (state.phase === "preparing" && !needsPassword) onPreparing?.();
  }, [state.phase, needsPassword, onClosed, onPreparing]);

  // After a refused or empty password, the cursor goes back to the box, as in every other form.
  // Each answer is a new state, so a second wrong guess moves the cursor back as the first did.
  useEffect(() => {
    if ((state.phase === "idle" && state.passwordProblem) || empty) focusField("password");
  }, [state, empty]);

  async function press(event?: FormEvent) {
    event?.preventDefault();
    if (state.phase === "starting") return;
    if (needsPassword && !password) return setEmpty(true);
    setEmpty(false);
    dispatch({ type: "press" });
    const result = await api.share.play(code, needsPassword ? password : undefined);
    if (result.ok) {
      const { playback } = result.data;
      dispatch({ type: "started", playlistUrl: playback.playlist_url, expiresAt: playback.expires_at, watermark: playback.watermark.text });
    } else {
      dispatch({ type: "refused", failure: result.error, retryWording: describeFailure });
    }
  }

  if (state.phase === "playing") {
    return (
      <Viewing
        code={code}
        playlistUrl={state.playlistUrl}
        expiresAt={state.expiresAt}
        watermark={state.watermark}
        title={title}
        control={control}
        onEnded={(notice) => dispatch({ type: "ended", notice })}
        onClosed={onClosed}
      />
    );
  }

  if (state.phase === "preparing") {
    // Only reached on screen for a protected link: for any other the page has taken over.
    const text = preparingText("processing");
    return (
      <div className="aspect-video border border-line bg-panel p-8">
        <div className="flex h-full flex-col items-start justify-center">
          <p className="label">Protected video</p>
          <h2 className="display mt-4 text-[1.9rem]">{text.heading}</h2>
          <p className="mt-3 max-w-md text-[0.95rem] leading-relaxed text-mute">This video isn't ready to watch yet. Try again in a minute.</p>
          <button type="button" onClick={() => dispatch({ type: "ended", notice: "" })} className="caps fillup fillup-ink mt-6 h-12 bg-silver px-8 text-ink">
            Try again
          </button>
        </div>
      </div>
    );
  }

  const idle = state.phase === "idle" ? state : null;
  const pending = state.phase === "starting";
  const problem = idle?.passwordProblem ?? (empty ? "Enter the password to watch." : undefined);

  return (
    <div className="border border-line bg-panel">
      <form onSubmit={press} noValidate className="flex aspect-video flex-col items-center justify-center gap-6 px-6 py-8 text-center">
        {idle?.notice ? (
          <div className="w-full max-w-sm text-left">
            <Notice tone="error">{idle.notice}</Notice>
          </div>
        ) : null}
        {needsPassword ? (
          <div className="w-full max-w-sm space-y-5 text-left">
            <p className="label text-center">This video is protected</p>
            <PasswordField id="password" label="Password" value={password} onChange={setPassword} error={problem} current autoFocus />
            <Submit pending={pending}>Unlock and play</Submit>
          </div>
        ) : (
          <>
            <button
              type="submit"
              disabled={pending}
              aria-label={`Play ${title}`}
              className="fillup fillup-ink flex size-20 items-center justify-center rounded-full bg-silver text-ink disabled:cursor-wait disabled:opacity-70"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false" className="ml-1 size-8">
                <path d="M7 4.5v15l12-7.5-12-7.5Z" fill="currentColor" />
              </svg>
            </button>
            {pending ? <p className="label">One moment…</p> : durationSeconds ? <p className="label">{clock(durationSeconds)}</p> : null}
          </>
        )}
      </form>
    </div>
  );
}

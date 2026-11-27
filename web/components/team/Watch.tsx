"use client";

import { useEffect, useRef, useState } from "react";

import { Notice } from "@/components/auth/controls";
import { describeFailure } from "@/components/auth/messages";
import { Watermark } from "@/components/share/Player";
import { api, type Schemas } from "@/lib/api/client";
import { ENDED, attachPlaylist } from "@/lib/hls";

type Playback = Schemas["PlaybackResponse"];
type State = { phase: "starting" } | { phase: "playing"; playback: Playback } | { phase: "over"; notice: string };

const quiet = "caps h-10 border border-line-hi px-4 text-paper/80 transition-colors hover:bg-raise hover:text-paper disabled:cursor-wait disabled:opacity-60";

/**
 * A signed-in viewing of a video the person may see (a recording in their workspace). It starts when it appears,
 * says it is still going every few seconds, and ends when it goes away. The server decides all of that: whenever
 * the reason to watch goes (taken out of the workspace, removed, the recording deleted) its next answer says so,
 * and the viewing stops with the sentence it gave. Every key the player needs comes from the same check, so a
 * player that holds one is out within the half minute the key lasts.
 */
export function Watch({ video, title, onClose }: { video: string; title: string; onClose: () => void }) {
  const [state, setState] = useState<State>({ phase: "starting" });
  const [attempt, setAttempt] = useState(0);
  const element = useRef<HTMLVideoElement>(null);
  const session = useRef<string | null>(null);

  // The viewing: started now, ended when this goes away (or is started again).
  useEffect(() => {
    let cancelled = false;
    setState({ phase: "starting" });
    void (async () => {
      const result = await api.playback.start(video);
      if (cancelled) {
        if (result.ok) void api.playback.stop(result.data.playback.session_id);
        return;
      }
      if (!result.ok) return setState({ phase: "over", notice: describeFailure(result.error) });
      session.current = result.data.playback.session_id;
      setState({ phase: "playing", playback: result.data.playback });
    })();
    return () => {
      cancelled = true;
      if (session.current) void api.playback.stop(session.current);
      session.current = null;
    };
  }, [video, attempt]);

  const playing = state.phase === "playing" ? state.playback : null;
  const over = (notice: string) => setState({ phase: "over", notice });

  useEffect(() => {
    const target = element.current;
    if (!playing || !target) return;
    return attachPlaylist(target, playing.playlist_url, over);
  }, [playing]);

  // Still going, and where it is. The answer is also how it finds out it is over, and why.
  useEffect(() => {
    if (!playing) return;
    const timer = setInterval(async () => {
      const result = await api.playback.heartbeat(playing.session_id, Math.floor(element.current?.currentTime ?? 0));
      if (result.ok) return;
      if (result.error.status === 409) over(result.error.message);
      else if (result.error.status === 404 || result.error.status === 403) over(ENDED);
    }, playing.heartbeat_interval_seconds * 1000);
    return () => clearInterval(timer);
  }, [playing]);

  // A viewing has a length. When it is up the server stops giving keys, so say so rather than stall.
  useEffect(() => {
    if (!playing) return;
    const timer = setTimeout(() => over(ENDED), Math.max(0, new Date(playing.expires_at).getTime() - Date.now()));
    return () => clearTimeout(timer);
  }, [playing]);

  return (
    <div className="w-full">
      {state.phase === "starting" ? (
        <div role="status" className="flex aspect-video items-center justify-center border border-line bg-panel">
          <p className="label">One moment…</p>
        </div>
      ) : state.phase === "over" ? (
        <div className="space-y-4">
          <Notice tone="error">{state.notice}</Notice>
          <button type="button" className={quiet} onClick={() => setAttempt((n) => n + 1)}>
            Watch again
          </button>
        </div>
      ) : (
        <div className="relative bg-black">
          <video ref={element} controls playsInline controlsList="nodownload" disablePictureInPicture aria-label={title} className="aspect-video w-full bg-black" />
          <Watermark text={state.playback.watermark.text} />
        </div>
      )}
      <button type="button" className={`${quiet} mt-3`} onClick={onClose}>
        Close
      </button>
    </div>
  );
}

"use client";

import Link from "next/link";
import { useCallback, useEffect, useState, type ReactNode } from "react";

import { Mark } from "@/components/site/Mark";
import { api } from "@/lib/api/client";

import { Player } from "./Player";
import { pollDelaySeconds, preparingText, viewOf, type ShareInfo, type View } from "./state";

export type Initial = { kind: "found"; share: ShareInfo } | { kind: "closed" };

function Page({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-dvh flex-col">
      <header className="flex h-14 items-center border-b border-line px-4 sm:px-6">
        <Link href="/" aria-label="Zeno, home" className="text-[0.82rem]">
          <Mark />
        </Link>
      </header>
      <main id="main" className="mx-auto w-full max-w-4xl flex-1 px-4 py-10 sm:px-6 sm:py-14">
        {children}
      </main>
      <footer className="border-t border-line px-4 py-6 text-[0.8125rem] text-mute sm:px-6">
        Shared with Zeno.{" "}
        <Link href="/register" className="text-paper underline decoration-line-hi underline-offset-4 hover:decoration-silver">
          Record and share your own
        </Link>
      </footer>
    </div>
  );
}

function Statement({ eyebrow, first, second, children }: { eyebrow?: string; first: string; second: string; children?: ReactNode }) {
  return (
    <>
      {eyebrow ? <p className="label">{eyebrow}</p> : null}
      <h1 className="display mt-4 text-[clamp(2rem,5vw,3.5rem)]">
        <span className="block">{first}</span>
        <span className="block">
          <em className="metal">{second}</em>
        </span>
      </h1>
      {children ? <div className="mt-6 max-w-lg text-[1rem] leading-relaxed text-mute">{children}</div> : null}
    </>
  );
}

/**
 * What a share link shows. The server rendered the first answer (and the page is reused for a few
 * seconds, so the API is asked about a link about four times a minute however many people open
 * it); from here the browser only asks again if the video is not ready yet, and slows down the
 * longer it waits.
 */
export function Share({ code, initial }: { code: string; initial: Initial }) {
  const [view, setView] = useState<View>(initial.kind === "closed" ? { kind: "closed" } : viewOf(initial.share));

  // A video that is not ready: ask again, politely, until it is (or the link closes).
  useEffect(() => {
    if (view.kind !== "preparing") return;
    let alive = true;
    let attempt = 0;
    let timer: ReturnType<typeof setTimeout>;
    const schedule = () => {
      timer = setTimeout(tick, pollDelaySeconds(attempt++) * 1000);
    };
    const tick = async () => {
      // Nobody is looking at a hidden tab: do not spend the API's attention on it.
      if (document.hidden) return schedule();
      const result = await api.share.resolve(code);
      if (!alive) return;
      if (result.ok) {
        const next = viewOf(result.data.share);
        if (next.kind === "preparing") schedule();
        else setView(next);
      } else if (result.error.status === 404 || result.error.status === 410) {
        setView({ kind: "closed" });
      } else {
        schedule();
      }
    };
    schedule();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [view.kind, code]);

  const preparing = useCallback(() => setView({ kind: "preparing", title: "This video", status: "processing" }), []);
  const closed = useCallback(() => setView({ kind: "closed" }), []);

  switch (view.kind) {
    case "closed":
      return (
        <Page>
          <Statement eyebrow="Shared link" first="This link is" second="no longer available.">
            <p>It may have been turned off, have run out, or the recording may have been deleted. Ask the person who sent it for a new one.</p>
          </Statement>
        </Page>
      );
    case "failed":
      return (
        <Page>
          <Statement eyebrow={view.title} first="This video couldn't" second="be prepared.">
            <p>Something went wrong while getting it ready to watch. Ask the person who shared it to upload it again.</p>
          </Statement>
        </Page>
      );
    case "preparing": {
      const text = preparingText(view.status);
      return (
        <Page>
          <Statement eyebrow={view.title} first={text.heading.split(" ").slice(0, -1).join(" ") || text.heading} second={text.heading.split(" ").slice(-1)[0] + "."}>
            <p role="status">{text.line}</p>
          </Statement>
        </Page>
      );
    }
    case "protected":
      return (
        <Page>
          <h1 className="display text-[clamp(1.8rem,4vw,3rem)]">A protected video</h1>
          <div className="mt-8">
            <Player code={code} title="Protected video" durationSeconds={null} needsPassword onClosed={closed} />
          </div>
        </Page>
      );
    case "ready":
      return (
        <Page>
          <h1 className="display text-[clamp(1.8rem,4vw,3rem)]">{view.title}</h1>
          <div className="mt-8">
            <Player code={code} title={view.title} durationSeconds={view.durationSeconds} needsPassword={false} onPreparing={preparing} onClosed={closed} />
          </div>
        </Page>
      );
  }
}

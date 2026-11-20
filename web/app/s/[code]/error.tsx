"use client";

import Link from "next/link";

import { Mark } from "@/components/site/Mark";

/** The API could not be asked. The page says so, and offers to try again; nothing is cached from this. */
export default function ShareError({ reset }: { error: Error; reset: () => void }) {
  return (
    <div className="flex min-h-dvh flex-col">
      <header className="flex h-14 items-center border-b border-line px-4 sm:px-6">
        <Link href="/" aria-label="Zeno, home" className="text-[0.82rem]">
          <Mark />
        </Link>
      </header>
      <main id="main" className="mx-auto w-full max-w-4xl flex-1 px-4 py-10 sm:px-6 sm:py-14">
        <p className="label">Shared link</p>
        <h1 className="display mt-4 text-[clamp(2rem,5vw,3.5rem)]">
          <span className="block">We couldn't load</span>
          <span className="block">
            <em className="metal">this video.</em>
          </span>
        </h1>
        <p role="alert" className="mt-6 max-w-lg text-[1rem] leading-relaxed text-mute">
          Something went wrong on our side. Try again in a moment.
        </p>
        <button type="button" onClick={reset} className="caps fillup fillup-ink mt-8 h-12 bg-silver px-8 text-ink">
          Try again
        </button>
      </main>
    </div>
  );
}

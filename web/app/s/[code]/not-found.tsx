import Link from "next/link";

import { Mark } from "@/components/site/Mark";

export default function NotFound() {
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
          <span className="block">We can't find</span>
          <span className="block">
            <em className="metal">that link.</em>
          </span>
        </h1>
        <p className="mt-6 max-w-lg text-[1rem] leading-relaxed text-mute">Check that the whole address was copied, or ask the person who sent it to send it again.</p>
      </main>
    </div>
  );
}

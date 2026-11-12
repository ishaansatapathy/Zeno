import { tape } from "@/content/site";

function Strip({ words, className, track }: { words: readonly string[]; className: string; track: string }) {
  // Two copies side by side; the track slides by exactly one copy, so the loop has no seam.
  const run = [...words, ...words];
  return (
    <div className={`overflow-hidden whitespace-nowrap py-2.5 ${className}`}>
      <div className={`flex w-max ${track}`}>
        {[0, 1].map((copy) => (
          <span key={copy} className="flex shrink-0">
            {run.map((w, i) => (
              <span key={`${copy}-${i}`} className="shout flex items-center text-[1.6rem] lg:text-[2rem]">
                <span className="px-5">{w}</span>
                <span aria-hidden="true">✦</span>
              </span>
            ))}
          </span>
        ))}
      </div>
    </div>
  );
}

/** Two strips of tape across the page, the way a crew marks a wall as theirs. */
export function Tape() {
  return (
    <div aria-hidden="true" className="relative -mx-4 my-8 h-36 overflow-hidden lg:my-12 lg:h-44">
      <Strip words={tape} className="absolute inset-x-[-5%] top-[38%] rotate-[2.4deg] bg-violet text-ink" track="animate-marquee-rev" />
      <Strip words={tape} className="absolute inset-x-[-5%] top-[22%] -rotate-[2.2deg] bg-spray text-ink shadow-[0_12px_30px_rgba(0,0,0,0.5)]" track="animate-marquee" />
    </div>
  );
}

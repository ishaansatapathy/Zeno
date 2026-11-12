import type { SVGProps } from "react";

/**
 * Marker doodles, drawn by hand to sit next to the ones on the cards: a crown, a sparkle, an
 * arrow, a scribbled underline and paint drips. All of them take `currentColor` and are
 * decoration only, so they are hidden from assistive technology.
 */
type Props = SVGProps<SVGSVGElement>;

const line = {
  fill: "none",
  stroke: "currentColor",
  strokeLinecap: "round",
  strokeLinejoin: "round",
} as const;

export function Crown(props: Props) {
  return (
    <svg viewBox="0 0 48 36" aria-hidden="true" focusable="false" {...props}>
      <g {...line} strokeWidth={2.6}>
        <path d="M7.5 31.5 5 11.2l9.6 9.4 9.2-15.8 8.4 16.1 10.6-9.9-2.1 20.3c-11-1.6-22.1-1.4-33.2.2Z" />
        <path d="M8.2 26.6c10.4-1.2 21.4-1.3 31.9.1" />
      </g>
      <g fill="currentColor">
        <circle cx="5" cy="9.6" r="2.1" />
        <circle cx="23.9" cy="3.4" r="2.1" />
        <circle cx="43" cy="9.8" r="2.1" />
      </g>
    </svg>
  );
}

export function Sparkle(props: Props) {
  return (
    <svg viewBox="0 0 40 40" aria-hidden="true" focusable="false" {...props}>
      <path
        d="M20 2.5c1.1 10.6 5.6 15.6 16.8 17.4-11 1.4-15.6 6.4-16.9 17.6-1.2-11.2-5.8-16-17.1-17.5C14.2 18.4 18.7 13.3 20 2.5Z"
        {...line}
        strokeWidth={2.4}
      />
    </svg>
  );
}

export function Arrow(props: Props) {
  return (
    <svg viewBox="0 0 96 48" aria-hidden="true" focusable="false" {...props}>
      <g {...line} strokeWidth={3.2}>
        <path d="M4 40c14-2 22-14 34-20 13-6.4 26-1.8 38-11.6" />
        <path d="M66 5.6 79 7.4l-4.4 12.4" />
      </g>
    </svg>
  );
}

/** A quick underline, the kind you put under a word you mean. */
export function Scribble(props: Props) {
  return (
    <svg viewBox="0 0 240 16" preserveAspectRatio="none" aria-hidden="true" focusable="false" {...props}>
      <path d="M3 10.5C46 4.2 96 3.8 140 6.4c34 2 64 1.4 97-3.6" {...line} strokeWidth={4} />
    </svg>
  );
}

/** Paint running down from a hard edge. Drips grow when the parent `group` is hovered. */
export function Drips({ className = "" }: { className?: string }) {
  const drops = [
    { x: 9, h: 7 },
    { x: 24, h: 13 },
    { x: 41, h: 5 },
    { x: 63, h: 15 },
    { x: 80, h: 8 },
    { x: 92, h: 11 },
  ];
  return (
    <svg
      viewBox="0 0 100 20"
      preserveAspectRatio="none"
      aria-hidden="true"
      focusable="false"
      className={`origin-top transition-transform duration-500 ease-out group-hover:scale-y-[1.55] ${className}`}
    >
      <g fill="currentColor">
        {drops.map((d) => (
          <g key={d.x}>
            <rect x={d.x - 1.6} y={-1} width={3.2} height={d.h} rx={1.6} />
            <ellipse cx={d.x} cy={d.h - 1} rx={2.2} ry={1.6} />
          </g>
        ))}
      </g>
    </svg>
  );
}

export function ArrowRight(props: Props) {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true" focusable="false" {...props}>
      <path d="M3 10h13M11 4.5 16.5 10 11 15.5" {...line} strokeWidth={2.4} />
    </svg>
  );
}

export function Person(props: Props) {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true" focusable="false" {...props}>
      <g {...line} strokeWidth={2}>
        <circle cx="10" cy="6.5" r="3.2" />
        <path d="M3.8 17c.9-3.4 3.3-5.2 6.2-5.2s5.3 1.8 6.2 5.2" />
      </g>
    </svg>
  );
}

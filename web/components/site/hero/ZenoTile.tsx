"use client";

import { m, useReducedMotion, useSpring } from "motion/react";
import dynamic from "next/dynamic";
import { useEffect, useRef, useState, type PointerEvent } from "react";

const ZenoTile3D = dynamic(() => import("./ZenoTile3D"), { ssr: false });

/** WebGL only where it is cheap: a fine pointer, a few cores, enough memory, and motion allowed. */
function capable(): boolean {
  if (typeof window === "undefined") return false;
  if (!window.matchMedia("(pointer: fine)").matches) return false;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return false;
  const nav = navigator as Navigator & { deviceMemory?: number };
  if ((nav.hardwareConcurrency ?? 4) < 4 || (nav.deviceMemory ?? 8) < 4) return false;
  try {
    return !!document.createElement("canvas").getContext("webgl2");
  } catch {
    return false;
  }
}

/** The same tile drawn in CSS: what phones, slow machines and reduced motion get. Never blank. */
function FlatTile({ tilt }: { tilt: boolean }) {
  const rx = useSpring(0, { stiffness: 120, damping: 14 });
  const ry = useSpring(0, { stiffness: 120, damping: 14 });
  const onMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!tilt) return;
    const box = event.currentTarget.getBoundingClientRect();
    ry.set(((event.clientX - box.left) / box.width - 0.5) * 22);
    rx.set(-((event.clientY - box.top) / box.height - 0.5) * 22);
  };
  return (
    <div className="grid h-full w-full place-items-center [perspective:900px]" onPointerMove={onMove} onPointerLeave={() => (rx.set(0), ry.set(0))}>
      <m.div
        style={{ rotateX: rx, rotateY: ry }}
        className="relative grid aspect-square w-[46%] place-items-center rounded-[24%] border-[1.5px] border-text bg-accent shadow-[7px_7px_0_0_rgb(var(--text))]"
      >
        <span className="absolute inset-[7%] rounded-[20%] bg-[radial-gradient(120%_90%_at_30%_20%,rgba(255,255,255,0.45),transparent_55%)]" />
        <svg viewBox="0 0 24 24" aria-hidden="true" className="relative h-[34%] w-[34%] translate-x-[6%] text-text">
          <path d="M6 3.8 20 12 6 20.2Z" fill="currentColor" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" />
        </svg>
      </m.div>
    </div>
  );
}

export function ZenoTile() {
  const reduce = useReducedMotion();
  const box = useRef<HTMLDivElement>(null);
  const [webgl, setWebgl] = useState(false);
  const [visible, setVisible] = useState(true);

  useEffect(() => setWebgl(capable()), []);
  useEffect(() => {
    const node = box.current;
    if (!node) return;
    const io = new IntersectionObserver(([entry]) => setVisible(entry.isIntersecting), { rootMargin: "120px" });
    io.observe(node);
    return () => io.disconnect();
  }, []);

  return (
    <div ref={box} className="relative h-full w-full">
      {/* A soft shadow on the paper, under whichever tile is showing. */}
      <span aria-hidden="true" className="absolute left-1/2 top-[72%] h-[9%] w-[44%] -translate-x-1/2 rounded-[50%] bg-text/20 blur-xl" />
      {webgl ? <ZenoTile3D active={visible} /> : <FlatTile tilt={!reduce} />}
    </div>
  );
}

"use client";

import { ReactLenis } from "lenis/react";
import { LazyMotion, MotionConfig, domAnimation, useReducedMotion } from "motion/react";
import type { ReactNode } from "react";

/**
 * Smooth, weighted scrolling on desktop pointers only. Touch keeps the platform's own scrolling
 * (anything else feels wrong under a thumb), and anyone who asked for less motion gets none: Lenis
 * is not mounted at all, and every Motion animation below becomes an instant change.
 *
 * Motion is loaded lean: components use `m.*` with only the DOM animation features, and `strict`
 * turns any stray full `motion.*` component into an error instead of a heavier page.
 */
export function SmoothScroll({ children }: { children: ReactNode }) {
  const reduce = useReducedMotion();
  return (
    <LazyMotion features={domAnimation} strict>
      <MotionConfig reducedMotion="user">
        {reduce ? (
          children
        ) : (
          <ReactLenis root options={{ lerp: 0.11, wheelMultiplier: 0.9, syncTouch: false, anchors: { offset: -72 } }}>
            {children}
          </ReactLenis>
        )}
      </MotionConfig>
    </LazyMotion>
  );
}

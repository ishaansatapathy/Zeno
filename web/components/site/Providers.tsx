"use client";

import { ReactLenis } from "lenis/react";
import { LazyMotion, MotionConfig, domMax, useReducedMotion } from "motion/react";
import type { ReactNode } from "react";

/**
 * Weighted, smooth scrolling for mouse and trackpad; touch keeps the platform's own scrolling.
 * Anyone who asked for less motion gets none: Lenis is not mounted, and every Motion animation
 * becomes an instant change. `strict` turns a stray full `motion.*` component into an error, so
 * only the lazily loaded `m.*` ones ship. The full feature set is needed for dragging the deck.
 * Anchor jumps land below the fixed nav through each section's own scroll-margin, which Lenis
 * honours, so native jumps (with Lenis off) land in the same place.
 */
export function Providers({ children }: { children: ReactNode }) {
  const reduce = useReducedMotion();
  return (
    <LazyMotion features={domMax} strict>
      <MotionConfig reducedMotion="user">
        {reduce ? (
          children
        ) : (
          <ReactLenis root options={{ lerp: 0.1, syncTouch: false, anchors: true }}>
            {children}
          </ReactLenis>
        )}
      </MotionConfig>
    </LazyMotion>
  );
}

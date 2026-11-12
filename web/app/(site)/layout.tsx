import type { ReactNode } from "react";

import { SiteFooter } from "@/components/site/SiteFooter";
import { SiteNav } from "@/components/site/SiteNav";
import { SmoothScroll } from "@/components/site/SmoothScroll";

/**
 * The public site. It is black and silver by design, so it asks for the dark palette whatever the
 * visitor's theme is (the app keeps following theirs).
 */
export default function SiteLayout({ children }: { children: ReactNode }) {
  return (
    <div data-palette="dark" className="min-h-dvh bg-bg text-text [color-scheme:dark]">
      <SmoothScroll>
        <SiteNav />
        <main id="main">{children}</main>
        <SiteFooter />
      </SmoothScroll>
    </div>
  );
}

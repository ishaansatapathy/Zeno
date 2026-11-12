import type { ReactNode } from "react";

import { SiteFooter } from "@/components/site/SiteFooter";
import { SiteNav } from "@/components/site/SiteNav";
import { SmoothScroll } from "@/components/site/SmoothScroll";

/**
 * The public site. It is designed for paper, so it asks for the light palette whatever the
 * visitor's theme is (the app keeps following theirs).
 */
export default function SiteLayout({ children }: { children: ReactNode }) {
  return (
    <div data-palette="light" className="paper min-h-dvh bg-bg text-text">
      <SmoothScroll>
        <SiteNav />
        <main id="main">{children}</main>
        <SiteFooter />
      </SmoothScroll>
    </div>
  );
}

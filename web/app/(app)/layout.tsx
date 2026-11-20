import type { Metadata } from "next";
import type { ReactNode } from "react";

import { AppFrame } from "@/components/app/AppFrame";
import { SessionProvider } from "@/lib/session";

export const metadata: Metadata = { robots: { index: false, follow: false } };

/** Everything behind a sign-in. */
export default function AppLayout({ children }: { children: ReactNode }) {
  return (
    <SessionProvider>
      <AppFrame>{children}</AppFrame>
    </SessionProvider>
  );
}

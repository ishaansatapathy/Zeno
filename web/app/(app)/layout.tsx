import { cookies } from "next/headers";
import type { ReactNode } from "react";

import { Header } from "@/components/shell/Header";
import { THEME_COOKIE } from "@/lib/theme/cookie";

/** The signed-in side and the account pages: the person's own theme, a plain frame. */
export default async function AppLayout({ children }: { children: ReactNode }) {
  const saved = (await cookies()).get(THEME_COOKIE)?.value;
  const theme = saved === "light" || saved === "dark" ? saved : "system";
  return (
    <div className="flex min-h-dvh flex-col bg-bg">
      <Header theme={theme} />
      <main id="main" className="mx-auto w-full max-w-5xl flex-1 px-4 py-10">
        {children}
      </main>
    </div>
  );
}

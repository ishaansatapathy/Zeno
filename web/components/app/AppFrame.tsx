"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useState, type ReactNode } from "react";

import { useSession } from "@/lib/session";

import { Mark } from "../site/Mark";

/**
 * The bar and the gate for the signed-in app. Nobody sees a screen before the API has said who
 * they are; someone who is not signed in is sent to sign in and brought back here afterwards.
 */
export function AppFrame({ children }: { children: ReactNode }) {
  const { session, reload, signOut } = useSession();
  const router = useRouter();
  const pathname = usePathname();
  const [leaving, setLeaving] = useState(false);

  useEffect(() => {
    if (session.status === "anonymous" && !leaving) router.replace(`/login?next=${encodeURIComponent(pathname)}`);
  }, [session.status, leaving, router, pathname]);

  if (session.status === "unavailable") {
    return (
      <main id="main" className="flex min-h-dvh flex-col items-center justify-center gap-5 px-4 text-center">
        <p role="alert" className="max-w-sm text-[0.95rem] text-mute">
          {session.failure.message}
        </p>
        <button type="button" onClick={() => void reload()} className="caps fillup fillup-ink h-12 bg-silver px-8 text-ink">
          Try again
        </button>
      </main>
    );
  }

  if (session.status !== "signed-in") {
    return (
      <main id="main" className="flex min-h-dvh items-center justify-center">
        <p className="label" role="status">
          Loading
        </p>
      </main>
    );
  }

  const { user } = session;
  return (
    <>
      <header className="fixed inset-x-2 top-2 z-50 flex h-12 items-stretch border border-line bg-ink/80 backdrop-blur-md sm:inset-x-3 sm:top-3">
        <Link href="/library" aria-label="Zeno, your library" className="flex items-center px-4 text-[0.82rem] sm:px-5">
          <Mark />
        </Link>
        <div className="ml-auto flex items-stretch">
          <Link
            href="/settings"
            aria-current={pathname === "/settings" ? "page" : undefined}
            aria-label={`Settings, ${user.name}`}
            title={user.email ?? undefined}
            className="caps fillup flex items-center border-l border-line px-5 text-paper/80"
          >
            <span className="hidden sm:inline">{user.name}</span>
            <span className="sm:hidden">Settings</span>
          </Link>
          <button
            type="button"
            onClick={() => {
              setLeaving(true);
              void signOut();
            }}
            className="caps fillup flex items-center border-l border-line px-5 text-paper/80"
          >
            Sign out
          </button>
        </div>
      </header>
      {children}
    </>
  );
}

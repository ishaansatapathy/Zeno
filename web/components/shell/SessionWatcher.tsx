"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";

import { SESSION_EXPIRED_EVENT } from "@/lib/api/instance";

/** When the session cannot be renewed anywhere in the app, the person is taken to sign in. */
export function SessionWatcher() {
  const router = useRouter();
  useEffect(() => {
    const onExpired = () => {
      const here = window.location.pathname;
      if (here !== "/sign-in" && here !== "/sign-up") router.push("/sign-in");
    };
    window.addEventListener(SESSION_EXPIRED_EVENT, onExpired);
    return () => window.removeEventListener(SESSION_EXPIRED_EVENT, onExpired);
  }, [router]);
  return null;
}

import { authApi } from "./auth";
import { createClient } from "./client";

/** Fired on `window` when the session cannot be renewed; the shell listens and goes to sign-in. */
export const SESSION_EXPIRED_EVENT = "zeno:session-expired";

export const api = createClient({
  baseUrl: process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080",
  onSessionExpired: () => {
    if (typeof window !== "undefined") window.dispatchEvent(new Event(SESSION_EXPIRED_EVENT));
  },
});

export const auth = authApi(api);

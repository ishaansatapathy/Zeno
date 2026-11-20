"use client";

import { useRouter } from "next/navigation";
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

import { api, type ApiFailure, type Schemas } from "./api/client";

export type User = Schemas["UserResponse"];

export type Session =
  | { status: "loading" }
  | { status: "anonymous" }
  | { status: "signed-in"; user: User }
  /** The API could not be asked. Not the same as signed out: a dropped connection must not look like a logout. */
  | { status: "unavailable"; failure: ApiFailure };

interface SessionApi {
  session: Session;
  /** Asks the API who is signed in. */
  reload: () => Promise<void>;
  /** Ends this session, here and on the server, and goes to the sign-in page. */
  signOut: () => Promise<void>;
}

const SessionContext = createContext<SessionApi | null>(null);

/** Who is signed in, asked of the API once when the app opens and kept for the screens beneath. */
export function SessionProvider({ children }: { children: ReactNode }) {
  const router = useRouter();
  const [session, setSession] = useState<Session>({ status: "loading" });

  const reload = useCallback(async () => {
    const result = await api.auth.me();
    if (result.ok) setSession({ status: "signed-in", user: result.data.user });
    else if (result.error.status === 401) setSession({ status: "anonymous" });
    else setSession({ status: "unavailable", failure: result.error });
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const signOut = useCallback(async () => {
    // Even if the server cannot be reached, this browser forgets the person: the cookies are
    // HttpOnly and expire on their own, and the next request would be refused if they had been
    // revoked. Better a sign-out that always works here than one that sometimes does nothing.
    await api.auth.logout();
    setSession({ status: "anonymous" });
    router.replace("/login?arrival=signed-out");
  }, [router]);

  const value = useMemo(() => ({ session, reload, signOut }), [session, reload, signOut]);
  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}

export function useSession(): SessionApi {
  const value = useContext(SessionContext);
  if (!value) throw new Error("useSession must be used inside <SessionProvider>");
  return value;
}

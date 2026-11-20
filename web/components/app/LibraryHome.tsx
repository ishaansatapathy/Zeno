"use client";

import { useSession } from "@/lib/session";

/** Where a signed-in person lands. The list of recordings arrives with the library screen. */
export function LibraryHome() {
  const { session } = useSession();
  if (session.status !== "signed-in") return null;
  const { user } = session;
  return (
    <>
      <p className="label">Signed in as {user.email}</p>
      <h1 className="display mt-6 text-[clamp(2.6rem,7vw,5.5rem)]">
        <span className="block">Your</span>
        <span className="block">
          <em className="metal">library.</em>
        </span>
      </h1>
      <p className="mt-8 max-w-md text-[1rem] leading-relaxed text-mute">Recordings you make will appear here.</p>
    </>
  );
}

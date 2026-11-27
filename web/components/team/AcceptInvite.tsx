"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";

import { Notice, Submit } from "@/components/auth/controls";
import { describeFailure } from "@/components/auth/messages";
import { api } from "@/lib/api/client";
import { useSession } from "@/lib/session";

/**
 * What an invitation email's link opens. Like the email-confirmation page it does nothing until the button
 * is pressed: mail scanners and link previews open links to look at them, and a link that joined on being
 * opened would be used up before the person arrived.
 */
export function AcceptInvite({ token }: { token: string }) {
  const router = useRouter();
  const { session, signOut } = useSession();
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<{ message: string; code: string } | null>(null);

  async function accept() {
    if (pending) return;
    setPending(true);
    setFailure(null);
    const result = await api.invites.accept(token);
    if (result.ok) {
      router.replace(`/team/${result.data.workspace.id}`);
      return;
    }
    setPending(false);
    setFailure({ code: result.error.code, message: describeFailure(result.error) });
  }

  return (
    <div className="mx-auto max-w-[26rem] border border-line bg-panel/95 p-7 sm:p-9">
      <p className="label">Invitation</p>
      <h1 className="display mt-6 text-[2.2rem]">Join a workspace.</h1>
      <p className="mt-4 text-[0.95rem] leading-relaxed text-mute">
        You are signed in as {session.status === "signed-in" ? session.user.email : "…"}. An invitation only works for the account that has the address it was sent to.
      </p>
      <div className="mt-8 space-y-5">
        {failure ? <Notice tone="error">{failure.message}</Notice> : null}
        {failure?.code === "wrong_account" ? (
          <button type="button" onClick={() => void signOut()} className="text-paper underline decoration-line-hi underline-offset-4 hover:decoration-silver">
            Sign out, then sign in with the address it was sent to
          </button>
        ) : null}
        {failure?.code === "invalid_token" ? (
          <p className="text-[0.875rem] text-mute">
            <Link href="/team" className="text-paper underline decoration-line-hi underline-offset-4 hover:decoration-silver">
              Your workspaces
            </Link>
          </p>
        ) : (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void accept();
            }}
          >
            <Submit pending={pending}>Accept invitation</Submit>
          </form>
        )}
      </div>
    </div>
  );
}

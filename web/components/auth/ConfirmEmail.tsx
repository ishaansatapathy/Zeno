"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";

import { api } from "@/lib/api/client";

import { AFTER_SIGN_IN } from "@/lib/next-path";

import { Notice, Submit } from "./controls";
import { describeFailure } from "./messages";

/**
 * The page an email link opens. It does nothing until the button is pressed: mail scanners and
 * link previews open links to look at them, and a link that confirmed on being opened would be
 * used up before the person arrived.
 */
export function ConfirmEmail({ token }: { token: string }) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<{ message: string; dead: boolean } | null>(null);

  async function confirm() {
    if (pending) return;
    setPending(true);
    setFailure(null);
    const result = await api.auth.verifyEmail(token);
    if (result.ok) {
      // Confirming signs the person in, so they go straight to their library.
      router.replace(AFTER_SIGN_IN);
      return;
    }
    setPending(false);
    setFailure({ message: describeFailure(result.error), dead: result.error.code === "invalid_token" });
  }

  return (
    <div className="space-y-5">
      {failure ? <Notice tone="error">{failure.message}</Notice> : null}
      {failure?.dead ? (
        <p className="text-[0.875rem] text-mute">
          <Link href="/verify-email" className="text-paper underline decoration-line-hi underline-offset-4 hover:decoration-silver">
            Get a new link
          </Link>
        </p>
      ) : (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void confirm();
          }}
        >
          <Submit pending={pending}>Confirm email</Submit>
        </form>
      )}
    </div>
  );
}

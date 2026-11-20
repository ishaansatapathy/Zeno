"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState, type FormEvent } from "react";

import { api } from "@/lib/api/client";
import { limits } from "@/lib/api/limits";

import { Notice, PasswordField, Submit, focusField } from "./controls";
import { describeFailure } from "./messages";
import { checkReset, firstProblem, type Problems } from "./validate";

const ORDER = ["new_password", "confirm"];

export function ResetForm({ token }: { token: string }) {
  const router = useRouter();
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [problems, setProblems] = useState<Problems>({});
  const [alert, setAlert] = useState<string | null>(null);
  const [dead, setDead] = useState(false);
  const [pending, setPending] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (pending) return;
    const found = checkReset({ password, confirm });
    setProblems(found);
    setAlert(null);
    if (Object.keys(found).length > 0) return focusField(firstProblem(found, ORDER));

    setPending(true);
    const result = await api.auth.resetPassword(token, password);
    if (result.ok) {
      // Every session was ended by the reset, so the next step is to sign in with the new password.
      router.replace("/login?arrival=reset");
      return;
    }
    setPending(false);
    const fields = result.error.fields;
    if (fields.new_password) {
      setProblems({ new_password: fields.new_password });
      focusField("new_password");
    } else if (result.error.code === "invalid_token") {
      setDead(true);
      setAlert(describeFailure(result.error));
    } else {
      setAlert(describeFailure(result.error));
    }
  }

  if (dead) {
    return (
      <div className="space-y-5">
        {alert ? <Notice tone="error">{alert}</Notice> : null}
        <p className="text-[0.875rem] text-mute">
          <Link href="/forgot-password" className="text-paper underline decoration-line-hi underline-offset-4 hover:decoration-silver">
            Ask for a new link
          </Link>
        </p>
      </div>
    );
  }

  return (
    <form onSubmit={submit} noValidate className="space-y-5">
      {alert ? <Notice tone="error">{alert}</Notice> : null}
      <PasswordField id="new_password" label="New password" value={password} onChange={setPassword} error={problems.new_password} hint={`At least ${limits.passwordMin} characters.`} autoFocus />
      <PasswordField id="confirm" label="Type it again" value={confirm} onChange={setConfirm} error={problems.confirm} />
      <Submit pending={pending}>Set new password</Submit>
    </form>
  );
}

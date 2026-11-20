"use client";

import { useRouter } from "next/navigation";
import { useState, type FormEvent } from "react";

import { api } from "@/lib/api/client";

import { Notice, PasswordField, ResendLink, Submit, TextField, focusField } from "./controls";
import { describeFailure } from "./messages";
import { checkLogin, firstProblem, type Problems } from "./validate";

const ORDER = ["email", "password"];

/** Notices that arrive with the page: the person was sent here by another screen. */
export type Arrival = "reset" | "expired" | "signed-out" | null;

const ARRIVALS: Record<Exclude<Arrival, null>, string> = {
  reset: "Your password is updated. Sign in with the new one.",
  expired: "Your session ended. Sign in to continue.",
  "signed-out": "You're signed out.",
};

export function LoginForm({ next, arrival }: { next: string; arrival: Arrival }) {
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [problems, setProblems] = useState<Problems>({});
  const [alert, setAlert] = useState<string | null>(null);
  const [unverified, setUnverified] = useState(false);
  const [pending, setPending] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (pending) return;
    const found = checkLogin({ email, password });
    setProblems(found);
    setAlert(null);
    setUnverified(false);
    if (Object.keys(found).length > 0) return focusField(firstProblem(found, ORDER));

    setPending(true);
    const result = await api.auth.login({ email: email.trim(), password });
    if (result.ok) {
      // Keep the button busy through the navigation, so a second click cannot start a second sign-in.
      router.replace(next);
      return;
    }
    setPending(false);
    if (result.error.code === "email_not_verified") {
      setUnverified(true);
      setAlert("Confirm your email address first. We sent you a link when you signed up.");
    } else {
      setAlert(describeFailure(result.error));
    }
  }

  return (
    <form onSubmit={submit} noValidate className="space-y-5">
      {arrival && !alert ? <Notice>{ARRIVALS[arrival]}</Notice> : null}
      {alert ? <Notice tone="error">{alert}</Notice> : null}
      <TextField id="email" label="Email" type="email" value={email} onChange={setEmail} error={problems.email} autoComplete="email" autoFocus />
      <PasswordField id="password" label="Password" value={password} onChange={setPassword} error={problems.password} current />
      <div className="pt-1">
        <Submit pending={pending}>Sign in</Submit>
      </div>
      {unverified ? <ResendLink email={email.trim()} intro="Lost the link?" /> : null}
    </form>
  );
}

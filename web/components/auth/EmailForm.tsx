"use client";

import { useState, type FormEvent } from "react";

import { api } from "@/lib/api/client";

import { Notice, Submit, TextField, focusField } from "./controls";
import { describeFailure } from "./messages";
import { checkEmail } from "./validate";

/**
 * One email box that asks for a link: to reset a password, or to confirm an address again. The
 * answer is the same whether or not an account exists, and the screen says only that.
 */
export function EmailForm({ kind, cta }: { kind: "reset" | "verify"; cta: string }) {
  const [email, setEmail] = useState("");
  const [problem, setProblem] = useState<string>();
  const [alert, setAlert] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [sentTo, setSentTo] = useState<string | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (pending) return;
    const found = checkEmail(email) ?? undefined;
    setProblem(found);
    setAlert(null);
    if (found) return focusField("email");

    setPending(true);
    const address = email.trim();
    const result = kind === "reset" ? await api.auth.forgotPassword(address) : await api.auth.resendVerification(address);
    setPending(false);
    if (result.ok) return setSentTo(address);
    if (result.error.fields.email) {
      setProblem(result.error.fields.email);
      return focusField("email");
    }
    setAlert(describeFailure(result.error));
  }

  if (sentTo) {
    return (
      <Notice>
        {kind === "reset" ? "If an account exists for " : "If "}
        <span className="text-paper">{sentTo}</span>
        {kind === "reset" ? ", we've sent instructions. The link works once and lasts an hour." : " still needs confirming, we've sent a new link."}
      </Notice>
    );
  }

  return (
    <form onSubmit={submit} noValidate className="space-y-5">
      {alert ? <Notice tone="error">{alert}</Notice> : null}
      <TextField id="email" label="Email" type="email" value={email} onChange={setEmail} error={problem} autoComplete="email" autoFocus />
      <Submit pending={pending}>{cta}</Submit>
    </form>
  );
}

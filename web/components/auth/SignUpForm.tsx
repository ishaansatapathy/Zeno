"use client";

import { useTranslations } from "next-intl";
import Link from "next/link";
import { useState, type FormEvent } from "react";

import { ErrorNotice } from "@/components/shell/ErrorNotice";
import { Button } from "@/components/ui/Button";
import { Field } from "@/components/ui/Field";
import { fieldMessages } from "@/lib/api/errors";
import { auth } from "@/lib/api/instance";
import { PASSWORD_MAX_CHARS, PASSWORD_MIN_CHARS, validateSignUp } from "@/lib/validation";

type Role = "student" | "creator";

export function SignUpForm() {
  const t = useTranslations();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);
  const [problems, setProblems] = useState<ReturnType<typeof validateSignUp>>({});

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy) return;
    const data = new FormData(event.currentTarget);
    const name = String(data.get("name") ?? "").trim();
    const email = String(data.get("email") ?? "").trim();
    const password = String(data.get("password") ?? "");
    const role: Role = data.get("role") === "creator" ? "creator" : "student";

    const found = validateSignUp({ name, email, password });
    setProblems(found);
    setFailure(null);
    if (Object.keys(found).length > 0) return;

    setBusy(true);
    try {
      await auth.register({ name, email, password, role });
      setDone(true);
    } catch (error) {
      setFailure(error);
    } finally {
      setBusy(false);
    }
  }

  if (done) {
    // The API gives the same answer whether or not the address was already registered, so this
    // page must not hint at either.
    return (
      <div role="status" className="flex flex-col gap-4">
        <p>{t("auth.signUp.checkEmail")}</p>
        <Link href="/sign-in" className="font-medium text-primary underline">
          {t("auth.signUp.signInLink")}
        </Link>
      </div>
    );
  }

  const fromServer = fieldMessages(failure);
  const say = (field: "name" | "email" | "password") => {
    const key = problems[field];
    if (!key) return fromServer[field];
    return t(`validation.${key}`, { min: PASSWORD_MIN_CHARS, max: PASSWORD_MAX_CHARS });
  };

  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-4">
      {failure !== null && <ErrorNotice error={failure} />}
      <Field name="name" label={t("auth.name")} autoComplete="name" required error={say("name")} />
      <Field
        name="email"
        type="email"
        label={t("auth.email")}
        autoComplete="email"
        autoCapitalize="none"
        spellCheck={false}
        inputMode="email"
        required
        error={say("email")}
      />
      <Field
        name="password"
        type="password"
        label={t("auth.password")}
        autoComplete="new-password"
        hint={t("auth.signUp.passwordHint", { min: PASSWORD_MIN_CHARS })}
        required
        error={say("password")}
      />
      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 text-sm font-medium">{t("auth.signUp.roleLabel")}</legend>
        {(["student", "creator"] as const).map((role) => (
          <label key={role} className="flex min-h-11 items-center gap-3 rounded-control border border-border-strong px-3">
            <input type="radio" name="role" value={role} defaultChecked={role === "student"} className="h-4 w-4 accent-primary" />
            <span>{t(role === "student" ? "auth.signUp.roleStudent" : "auth.signUp.roleCreator")}</span>
          </label>
        ))}
      </fieldset>
      <Button type="submit" loading={busy} size="lg">
        {t("auth.signUp.submit")}
      </Button>
      <p className="text-center text-sm text-text-muted">
        {t("auth.signUp.haveAccount")}{" "}
        <Link href="/sign-in" className="font-medium text-primary underline">
          {t("auth.signUp.signInLink")}
        </Link>
      </p>
    </form>
  );
}

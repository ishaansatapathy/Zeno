"use client";

import { useTranslations } from "next-intl";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState, type FormEvent } from "react";

import { ErrorNotice } from "@/components/shell/ErrorNotice";
import { Button } from "@/components/ui/Button";
import { Field } from "@/components/ui/Field";
import { fieldMessages } from "@/lib/api/errors";
import { auth } from "@/lib/api/instance";
import { validateSignIn } from "@/lib/validation";

export function SignInForm() {
  const t = useTranslations();
  const router = useRouter();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);
  const [problems, setProblems] = useState<ReturnType<typeof validateSignIn>>({});

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy) return;
    const data = new FormData(event.currentTarget);
    const email = String(data.get("email") ?? "").trim();
    const password = String(data.get("password") ?? "");

    const found = validateSignIn({ email, password });
    setProblems(found);
    setFailure(null);
    if (Object.keys(found).length > 0) return;

    setBusy(true);
    try {
      await auth.login(email, password);
      router.push("/account");
    } catch (error) {
      setFailure(error);
    } finally {
      setBusy(false);
    }
  }

  const fromServer = fieldMessages(failure);
  const say = (field: "email" | "password") => {
    const key = problems[field];
    return key ? t(`validation.${key}`) : fromServer[field];
  };

  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-4">
      {failure !== null && <ErrorNotice error={failure} />}
      <Field
        name="email"
        type="email"
        label={t("auth.email")}
        autoComplete="username"
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
        autoComplete="current-password"
        required
        error={say("password")}
      />
      <Button type="submit" loading={busy} size="lg">
        {t("auth.signIn.submit")}
      </Button>
      <p className="text-center text-sm text-text-muted">
        {t("auth.signIn.noAccount")}{" "}
        <Link href="/sign-up" className="font-medium text-primary underline">
          {t("auth.signIn.createLink")}
        </Link>
      </p>
    </form>
  );
}

"use client";

import { useTranslations } from "next-intl";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";

import { ErrorNotice } from "@/components/shell/ErrorNotice";
import { Button } from "@/components/ui/Button";
import { Card } from "@/components/ui/Card";
import { isApiError, isNetworkError } from "@/lib/api/errors";
import { auth } from "@/lib/api/instance";
import type { User } from "@/lib/api/auth";

type State = { status: "loading" } | { status: "ready"; user: User } | { status: "failed"; error: unknown };

export function AccountView() {
  const t = useTranslations();
  const router = useRouter();
  const [state, setState] = useState<State>({ status: "loading" });
  const [leaving, setLeaving] = useState(false);

  useEffect(() => {
    const controller = new AbortController();
    auth
      .me(controller.signal)
      .then(({ user }) => setState({ status: "ready", user }))
      .catch((error: unknown) => {
        // Leaving the page cancels the call; that is not a failure to show.
        if (isNetworkError(error) && error.reason === "aborted") return;
        // Not signed in (and not renewable): the watcher is already taking them to sign in.
        if (isApiError(error) && error.status === 401) {
          router.replace("/sign-in");
          return;
        }
        setState({ status: "failed", error });
      });
    return () => controller.abort();
  }, [router]);

  async function signOut() {
    setLeaving(true);
    try {
      await auth.logout();
    } catch {
      // Whether or not the server heard us, leave: the cookies are the server's to clear.
    }
    router.push("/");
    router.refresh();
  }

  if (state.status === "loading") return <p role="status">{t("common.loading")}</p>;
  if (state.status === "failed") return <ErrorNotice error={state.error} />;

  const { user } = state;
  return (
    <Card className="flex flex-col gap-4">
      <h2 className="text-xl font-semibold">{t("account.welcome", { name: user.name })}</h2>
      <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
        <dt className="text-text-muted">{t("account.email")}</dt>
        <dd>{user.email}</dd>
        <dt className="text-text-muted">{t("account.role")}</dt>
        <dd>{t(`account.roles.${user.role}`)}</dd>
        <dt className="sr-only">{t("account.verified")}</dt>
        <dd className="col-span-2 font-medium">{user.email_verified ? t("account.verified") : t("account.notVerified")}</dd>
      </dl>
      <div>
        <Button variant="secondary" onClick={signOut} loading={leaving}>
          {t("nav.signOut")}
        </Button>
      </div>
    </Card>
  );
}

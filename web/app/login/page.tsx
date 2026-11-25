import type { Metadata } from "next";

import { AuthShell, FooterLink } from "@/components/auth/AuthShell";
import { LoginForm, type Arrival } from "@/components/auth/LoginForm";
import { noindex } from "@/lib/noindex";
import { safeNext } from "@/lib/next-path";

export const metadata: Metadata = { title: "Sign in · Zeno", robots: noindex };

const ARRIVALS: Arrival[] = ["reset", "expired", "signed-out", "password-changed"];

export default async function Page({ searchParams }: { searchParams: Promise<Record<string, string | string[] | undefined>> }) {
  const query = await searchParams;
  const arrival = ARRIVALS.find((name) => name === query.arrival) ?? null;
  return (
    <AuthShell
      title={["Welcome", "back."]}
      intro="Sign in to your library."
      footer={
        <>
          New to Zeno? <FooterLink href="/register">Create an account</FooterLink>
          <span aria-hidden="true"> · </span>
          <FooterLink href="/forgot-password">Forgot your password?</FooterLink>
        </>
      }
    >
      <LoginForm next={safeNext(query.next)} arrival={arrival} />
    </AuthShell>
  );
}

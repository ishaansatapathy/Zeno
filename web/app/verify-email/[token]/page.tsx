import type { Metadata } from "next";

import { AuthShell } from "@/components/auth/AuthShell";
import { ConfirmEmail } from "@/components/auth/ConfirmEmail";
import { noindex } from "@/lib/noindex";

export const metadata: Metadata = { title: "Confirm your email · Zeno", robots: noindex };

export default async function Page({ params }: { params: Promise<{ token: string }> }) {
  const { token } = await params;
  return (
    <AuthShell title={["Confirm your", "email."]} intro="One last step: confirm this address to finish creating your account.">
      <ConfirmEmail token={token} />
    </AuthShell>
  );
}

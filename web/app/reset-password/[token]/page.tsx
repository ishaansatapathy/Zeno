import type { Metadata } from "next";

import { AuthShell } from "@/components/auth/AuthShell";
import { ResetForm } from "@/components/auth/ResetForm";
import { noindex } from "@/lib/noindex";

export const metadata: Metadata = { title: "Choose a new password · Zeno", robots: noindex };

export default async function Page({ params }: { params: Promise<{ token: string }> }) {
  const { token } = await params;
  return (
    <AuthShell title={["Choose a new", "password."]} intro="Setting a new password signs you out everywhere else.">
      <ResetForm token={token} />
    </AuthShell>
  );
}

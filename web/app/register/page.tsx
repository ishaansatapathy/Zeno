import type { Metadata } from "next";

import { AuthShell, FooterLink } from "@/components/auth/AuthShell";
import { RegisterForm } from "@/components/auth/RegisterForm";
import { noindex } from "@/lib/noindex";

export const metadata: Metadata = { title: "Create an account · Zeno", robots: noindex };

export default function Page() {
  return (
    <AuthShell
      title={["Start", "recording."]}
      intro="Make an account to record, share and keep your videos."
      footer={
        <>
          Already have an account? <FooterLink href="/login">Sign in</FooterLink>
        </>
      }
    >
      <RegisterForm />
    </AuthShell>
  );
}

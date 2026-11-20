import type { Metadata } from "next";

import { AuthShell, FooterLink } from "@/components/auth/AuthShell";
import { EmailForm } from "@/components/auth/EmailForm";
import { noindex } from "@/lib/noindex";

export const metadata: Metadata = { title: "Confirm your email · Zeno", robots: noindex };

export default function Page() {
  return (
    <AuthShell
      title={["Confirm your", "email."]}
      intro="Enter the address you signed up with and we'll send a new link."
      footer={
        <>
          Already confirmed? <FooterLink href="/login">Sign in</FooterLink>
        </>
      }
    >
      <EmailForm kind="verify" cta="Send link" />
    </AuthShell>
  );
}

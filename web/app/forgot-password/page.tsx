import type { Metadata } from "next";

import { AuthShell, FooterLink } from "@/components/auth/AuthShell";
import { EmailForm } from "@/components/auth/EmailForm";
import { noindex } from "@/lib/noindex";

export const metadata: Metadata = { title: "Reset your password · Zeno", robots: noindex };

export default function Page() {
  return (
    <AuthShell
      title={["Reset your", "password."]}
      intro="Enter your email and we'll send a link to choose a new one."
      footer={
        <>
          Remembered it? <FooterLink href="/login">Sign in</FooterLink>
        </>
      }
    >
      <EmailForm kind="reset" cta="Send link" />
    </AuthShell>
  );
}

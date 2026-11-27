import type { Metadata } from "next";

import { AcceptInvite } from "@/components/team/AcceptInvite";
import { noindex } from "@/lib/noindex";

export const metadata: Metadata = { title: "Join a workspace · Zeno", robots: noindex };

export default async function Page({ params }: { params: Promise<{ token: string }> }) {
  const { token } = await params;
  return (
    <main id="main" className="px-4 pb-24 pt-36 sm:px-6">
      <AcceptInvite token={token} />
    </main>
  );
}

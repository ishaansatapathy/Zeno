import type { Metadata } from "next";

import { Teams } from "@/components/team/Teams";

export const metadata: Metadata = { title: "Your workspaces · Zeno" };

export default function Page() {
  return (
    <main id="main" className="mx-auto max-w-page px-4 pb-24 pt-32 sm:px-6">
      <Teams />
    </main>
  );
}

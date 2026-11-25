import type { Metadata } from "next";

import { Settings } from "@/components/settings/Settings";

export const metadata: Metadata = { title: "Your settings · Zeno" };

export default function Page() {
  return (
    <main id="main" className="mx-auto max-w-page px-4 pb-24 pt-32 sm:px-6">
      <Settings />
    </main>
  );
}

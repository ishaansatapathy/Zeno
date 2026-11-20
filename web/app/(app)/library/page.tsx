import type { Metadata } from "next";

import { LibraryHome } from "@/components/app/LibraryHome";

export const metadata: Metadata = { title: "Your library · Zeno" };

export default function Page() {
  return (
    <main id="main" className="mx-auto max-w-page px-4 pb-24 pt-32 sm:px-6">
      <LibraryHome />
    </main>
  );
}

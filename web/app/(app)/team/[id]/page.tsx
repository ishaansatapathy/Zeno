import type { Metadata } from "next";

import { WorkspaceView } from "@/components/team/Workspace";

export const metadata: Metadata = { title: "Workspace · Zeno" };

export default async function Page({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  return (
    <main id="main" className="mx-auto max-w-page px-4 pb-24 pt-32 sm:px-6">
      <WorkspaceView id={id} />
    </main>
  );
}

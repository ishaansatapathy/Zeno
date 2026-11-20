import type { Metadata } from "next";
import { notFound } from "next/navigation";

import { Share } from "@/components/share/Share";
import { resolveShare } from "@/lib/api/server";
import { noindex } from "@/lib/noindex";

// A rendered share page is reused for 15 seconds, however many people open it: the API is asked
// about a link about four times a minute, not once per visitor. The API says the same about its own
// answer (`Cache-Control: public, max-age=15`), and playing asks the database again, so a page
// reused after a revoke shows a player that is refused, never a video that plays. It must be a
// literal for Next to read it, so it matches SHARE_REVALIDATE_SECONDS by hand.
export const revalidate = 15;

// No link is known at build time. An empty list turns on rendering a page the first time its link
// is asked for, and reusing it after that.
export function generateStaticParams() {
  return [];
}

export async function generateMetadata({ params }: { params: Promise<{ code: string }> }): Promise<Metadata> {
  const { code } = await params;
  const result = await resolveShare(code);
  const base = { robots: noindex };
  if (result.kind === "missing") return { ...base, title: "Link not found · Zeno" };
  if (result.kind === "closed") return { ...base, title: "This link is no longer available · Zeno" };
  const { share } = result;
  // A protected link reveals nothing about the video, and so does its preview.
  const title = share.requires_password ? "A protected video" : (share.title ?? "A video");
  const description = share.requires_password ? "Enter the password to watch." : "Watch this video, shared with you on Zeno.";
  return {
    ...base,
    title: `${title} · Zeno`,
    description,
    openGraph: { title, description, siteName: "Zeno", type: "video.other" },
    twitter: { card: "summary_large_image", title, description },
  };
}

export default async function Page({ params }: { params: Promise<{ code: string }> }) {
  const { code } = await params;
  const result = await resolveShare(code);
  if (result.kind === "missing") notFound();
  return <Share code={code} initial={result} />;
}

import { readFile } from "node:fs/promises";
import path from "node:path";

import { ImageResponse } from "next/og";

import { resolveShare } from "@/lib/api/server";

export const alt = "A video shared on Zeno";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";
// Same window as the page, so a preview never shows a title the page no longer does by much.
export const revalidate = 15;

/** What the card says about a link. A protected or closed link says nothing about the video. */
async function words(code: string): Promise<{ eyebrow: string; title: string }> {
  const result = await resolveShare(code);
  if (result.kind === "missing") return { eyebrow: "Shared link", title: "We can't find that link" };
  if (result.kind === "closed") return { eyebrow: "Shared link", title: "This link is no longer available" };
  if (result.share.requires_password) return { eyebrow: "Shared on Zeno", title: "A protected video" };
  const minutes = result.share.duration_seconds ? ` · ${Math.max(1, Math.round(result.share.duration_seconds / 60))} min` : "";
  return { eyebrow: `Shared on Zeno${minutes}`, title: result.share.title ?? "A video" };
}

/** Titles are cut so the card never overflows: about three lines of large type. */
function fit(title: string): string {
  const chars = [...title];
  return chars.length > 90 ? `${chars.slice(0, 89).join("").trimEnd()}…` : title;
}

export default async function Image({ params }: { params: Promise<{ code: string }> }) {
  const { code } = await params;
  const { eyebrow, title } = await words(code);
  const serif = await readFile(path.join(process.cwd(), "node_modules/@fontsource/instrument-serif/files/instrument-serif-latin-400-normal.woff"));
  const text = fit(title);

  return new ImageResponse(
    (
      <div style={{ width: "100%", height: "100%", display: "flex", flexDirection: "column", justifyContent: "space-between", background: "#000000", color: "#f2f3f5", padding: 72, fontFamily: "Instrument Serif" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 18 }}>
          <div style={{ width: 44, height: 44, background: "#e4e7eb", display: "flex", alignItems: "center", justifyContent: "center" }}>
            <div style={{ width: 0, height: 0, marginLeft: 6, borderTop: "11px solid transparent", borderBottom: "11px solid transparent", borderLeft: "17px solid #000000" }} />
          </div>
          <div style={{ fontSize: 44, letterSpacing: -1 }}>Zeno</div>
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 28 }}>
          <div style={{ fontSize: 28, color: "#8e939a", letterSpacing: 4, textTransform: "uppercase" }}>{eyebrow}</div>
          <div style={{ fontSize: text.length > 48 ? 72 : 96, lineHeight: 1.05, letterSpacing: -2, color: "#e4e7eb" }}>{text}</div>
        </div>
      </div>
    ),
    { ...size, fonts: [{ name: "Instrument Serif", data: serif, style: "normal", weight: 400 }] },
  );
}

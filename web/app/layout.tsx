import "./globals.css";

import type { Metadata, Viewport } from "next";
import localFont from "next/font/local";
import type { ReactNode } from "react";

// next/font reads these paths at build time, so each has to be a plain string.
/** Body: Barlow (OFL). */
const body = localFont({
  src: [
    { path: "../node_modules/@fontsource/barlow/files/barlow-latin-400-normal.woff2", weight: "400", style: "normal" },
    { path: "../node_modules/@fontsource/barlow/files/barlow-latin-500-normal.woff2", weight: "500", style: "normal" },
    { path: "../node_modules/@fontsource/barlow/files/barlow-latin-600-normal.woff2", weight: "600", style: "normal" },
  ],
  variable: "--font-body",
  display: "swap",
});

/** Headlines and labels: Barlow Condensed (OFL), heavy italic for shouting, semibold for labels. */
const cond = localFont({
  src: [
    { path: "../node_modules/@fontsource/barlow-condensed/files/barlow-condensed-latin-600-normal.woff2", weight: "600", style: "normal" },
    { path: "../node_modules/@fontsource/barlow-condensed/files/barlow-condensed-latin-600-italic.woff2", weight: "600", style: "italic" },
    { path: "../node_modules/@fontsource/barlow-condensed/files/barlow-condensed-latin-800-normal.woff2", weight: "800", style: "normal" },
    { path: "../node_modules/@fontsource/barlow-condensed/files/barlow-condensed-latin-800-italic.woff2", weight: "800", style: "italic" },
  ],
  variable: "--font-cond",
  display: "swap",
});

/** The sprayed word: Sedgwick Ave Display (OFL), a graffiti handstyle. */
const tag = localFont({
  src: [{ path: "../node_modules/@fontsource/sedgwick-ave-display/files/sedgwick-ave-display-latin-400-normal.woff2", weight: "400", style: "normal" }],
  variable: "--font-tag",
  display: "swap",
});

export const metadata: Metadata = {
  title: "Zeno: teach what you know, keep what you earn",
  description:
    "Video courses and live classes for independent educators. Every lesson locked segment by segment, every sale in a ledger, every payout to your bank.",
};

export const viewport: Viewport = { themeColor: "#0c0a0f", colorScheme: "dark" };

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${body.variable} ${cond.variable} ${tag.variable}`}>
      <body className="grit">
        <a href="#main" className="caps sr-only bg-spray px-4 py-2 text-ink focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-[70]">
          Skip to content
        </a>
        {children}
      </body>
    </html>
  );
}

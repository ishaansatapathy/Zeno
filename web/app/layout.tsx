import "./globals.css";

import type { Metadata, Viewport } from "next";
import localFont from "next/font/local";
import type { ReactNode } from "react";

// next/font reads these paths at build time, so each has to be a plain string. All three families
// are under the SIL Open Font License.

/** Statements: Instrument Serif, upright and italic. */
const serif = localFont({
  src: [
    { path: "../node_modules/@fontsource/instrument-serif/files/instrument-serif-latin-400-normal.woff2", weight: "400", style: "normal" },
    { path: "../node_modules/@fontsource/instrument-serif/files/instrument-serif-latin-400-italic.woff2", weight: "400", style: "italic" },
  ],
  variable: "--font-serif",
  display: "swap",
});

/** Reading text, navigation and buttons: Instrument Sans, its companion. */
const sans = localFont({
  src: [{ path: "../node_modules/@fontsource-variable/instrument-sans/files/instrument-sans-latin-wght-normal.woff2", weight: "400 700", style: "normal" }],
  variable: "--font-sans",
  display: "swap",
});

/** Labels, logs and figures: IBM Plex Mono. */
const mono = localFont({
  src: [
    { path: "../node_modules/@fontsource/ibm-plex-mono/files/ibm-plex-mono-latin-400-normal.woff2", weight: "400", style: "normal" },
    { path: "../node_modules/@fontsource/ibm-plex-mono/files/ibm-plex-mono-latin-500-normal.woff2", weight: "500", style: "normal" },
  ],
  variable: "--font-mono",
  display: "swap",
});

// Link previews need absolute addresses for their images. Where the site lives is `SITE_URL` (the
// same address the API calls `CLIENT_URL`); without it, a preview's image would point at localhost.
const siteUrl = process.env.SITE_URL ?? "http://localhost:3000";

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl),
  title: "Zeno: record it once, share it anywhere",
  description:
    "One place to record, share, host and sell video, for creators, teachers and teams. A link before you stop recording, encrypted playback on any connection, and courses with payouts. Live streaming, an editor and meetings are on the way.",
};

export const viewport: Viewport = { themeColor: "#000000", colorScheme: "dark" };

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${serif.variable} ${sans.variable} ${mono.variable}`}>
      <body>
        <a href="#main" className="caps sr-only bg-silver px-4 py-3 text-ink focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-[70]">
          Skip to content
        </a>
        {children}
      </body>
    </html>
  );
}

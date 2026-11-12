import "./globals.css";

import type { Metadata, Viewport } from "next";
import localFont from "next/font/local";
import type { ReactNode } from "react";

// next/font reads these paths at build time, so each has to be a plain string. All four
// families are under the SIL Open Font License.

/** Statements: PT Serif, upright and italic. */
const serif = localFont({
  src: [
    { path: "../node_modules/@fontsource/pt-serif/files/pt-serif-latin-400-normal.woff2", weight: "400", style: "normal" },
    { path: "../node_modules/@fontsource/pt-serif/files/pt-serif-latin-400-italic.woff2", weight: "400", style: "italic" },
  ],
  variable: "--font-serif",
  display: "swap",
});

/** Body, navigation and buttons: Inter Tight. */
const sans = localFont({
  src: [
    { path: "../node_modules/@fontsource/inter-tight/files/inter-tight-latin-400-normal.woff2", weight: "400", style: "normal" },
    { path: "../node_modules/@fontsource/inter-tight/files/inter-tight-latin-500-normal.woff2", weight: "500", style: "normal" },
    { path: "../node_modules/@fontsource/inter-tight/files/inter-tight-latin-600-normal.woff2", weight: "600", style: "normal" },
  ],
  variable: "--font-sans",
  display: "swap",
});

/** The few giant words (FAQ, the footer's name): Anton, tall and condensed. */
const cond = localFont({
  src: [{ path: "../node_modules/@fontsource/anton/files/anton-latin-400-normal.woff2", weight: "400", style: "normal" }],
  variable: "--font-cond",
  display: "swap",
});

/** Labels, logs and the stream of characters in the hero: IBM Plex Mono. */
const mono = localFont({
  src: [
    { path: "../node_modules/@fontsource/ibm-plex-mono/files/ibm-plex-mono-latin-400-normal.woff2", weight: "400", style: "normal" },
    { path: "../node_modules/@fontsource/ibm-plex-mono/files/ibm-plex-mono-latin-500-normal.woff2", weight: "500", style: "normal" },
  ],
  variable: "--font-mono",
  display: "swap",
});

export const metadata: Metadata = {
  title: "Zeno: teach what you know, keep what you earn",
  description:
    "Video courses and live classes for independent educators. Every lesson locked segment by segment, every sale counted once, every payout to your bank.",
};

export const viewport: Viewport = { themeColor: "#050505", colorScheme: "dark" };

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${serif.variable} ${sans.variable} ${cond.variable} ${mono.variable}`}>
      <body>
        <a href="#main" className="caps sr-only bg-signal px-4 py-3 text-ink focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-[70]">
          Skip to content
        </a>
        {children}
      </body>
    </html>
  );
}

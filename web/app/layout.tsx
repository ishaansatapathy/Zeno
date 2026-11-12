import "./globals.css";

import type { Metadata, Viewport } from "next";
import localFont from "next/font/local";
import type { ReactNode } from "react";

// next/font reads these paths at build time, so each has to be a plain string. Both families are
// under the SIL Open Font License.

/** Everything you read: Geist, a plain, exact grotesk. */
const sans = localFont({
  src: [{ path: "../node_modules/@fontsource-variable/geist/files/geist-latin-wght-normal.woff2", weight: "100 900", style: "normal" }],
  variable: "--font-sans",
  display: "swap",
});

/** Labels, logs and figures: Geist Mono. */
const mono = localFont({
  src: [{ path: "../node_modules/@fontsource-variable/geist-mono/files/geist-mono-latin-wght-normal.woff2", weight: "100 900", style: "normal" }],
  variable: "--font-mono",
  display: "swap",
});

export const metadata: Metadata = {
  title: "Zeno: teach what you know, keep what you earn",
  description:
    "Video courses and live classes for independent educators. Every lesson locked segment by segment, every sale counted once, every payout to your bank.",
};

export const viewport: Viewport = { themeColor: "#09090b", colorScheme: "dark" };

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${sans.variable} ${mono.variable}`}>
      <body>
        <a href="#main" className="sr-only rounded-md bg-paper px-4 py-2.5 text-sm font-medium text-ink focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-[70]">
          Skip to content
        </a>
        {children}
      </body>
    </html>
  );
}

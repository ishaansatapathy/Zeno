import "./globals.css";

import { GeistMono } from "geist/font/mono";
import { GeistSans } from "geist/font/sans";
import type { Metadata, Viewport } from "next";
import { NextIntlClientProvider } from "next-intl";
import { getLocale, getMessages, getTranslations } from "next-intl/server";
import localFont from "next/font/local";
import { cookies } from "next/headers";
import type { ReactNode } from "react";

import { SessionWatcher } from "@/components/shell/SessionWatcher";
import { ToastProvider } from "@/components/ui/Toast";
import { THEME_COOKIE } from "@/lib/theme/cookie";
import { themeCss } from "@/lib/theme/tokens";

/** The display face: Instrument Serif (OFL), self-hosted, with its italic for the one word that matters. */
const display = localFont({
  src: [
    { path: "../node_modules/@fontsource/instrument-serif/files/instrument-serif-latin-400-normal.woff2", style: "normal", weight: "400" },
    { path: "../node_modules/@fontsource/instrument-serif/files/instrument-serif-latin-400-italic.woff2", style: "italic", weight: "400" },
  ],
  variable: "--font-display",
  display: "swap",
  fallback: ["Georgia", "Times New Roman", "serif"],
});

export async function generateMetadata(): Promise<Metadata> {
  const t = await getTranslations("meta");
  return { title: { default: t("title"), template: "%s · Zeno" }, description: t("description") };
}

export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#f4efe6" },
    { media: "(prefers-color-scheme: dark)", color: "#13110d" },
  ],
};

export default async function RootLayout({ children }: { children: ReactNode }) {
  const [locale, messages, t, jar] = await Promise.all([getLocale(), getMessages(), getTranslations("common"), cookies()]);
  const saved = jar.get(THEME_COOKIE)?.value;
  const theme = saved === "light" || saved === "dark" ? saved : undefined;

  return (
    <html
      lang={locale}
      data-theme={theme}
      className={`${GeistSans.variable} ${GeistMono.variable} ${display.variable}`}
      suppressHydrationWarning
    >
      <head>
        {/* The palette, from the tokens: the one place colours are defined. */}
        <style dangerouslySetInnerHTML={{ __html: themeCss() }} />
      </head>
      <body>
        <NextIntlClientProvider locale={locale} messages={messages}>
          <ToastProvider dismissLabel={t("close")}>
            <a
              href="#main"
              className="sr-only rounded-control bg-primary px-4 py-2 text-on-primary focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-[60]"
            >
              {t("skipToContent")}
            </a>
            {children}
            <SessionWatcher />
          </ToastProvider>
        </NextIntlClientProvider>
      </body>
    </html>
  );
}

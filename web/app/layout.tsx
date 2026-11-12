import "./globals.css";

import type { Metadata } from "next";
import { NextIntlClientProvider } from "next-intl";
import { getLocale, getMessages, getTranslations } from "next-intl/server";
import { cookies } from "next/headers";
import type { ReactNode } from "react";

import { Header } from "@/components/shell/Header";
import { SessionWatcher } from "@/components/shell/SessionWatcher";
import { ToastProvider } from "@/components/ui/Toast";
import { THEME_COOKIE } from "@/lib/theme/cookie";
import { themeCss } from "@/lib/theme/tokens";

export async function generateMetadata(): Promise<Metadata> {
  const t = await getTranslations("meta");
  return { title: { default: t("title"), template: "%s · Zeno" }, description: t("description") };
}

export default async function RootLayout({ children }: { children: ReactNode }) {
  const [locale, messages, t] = await Promise.all([getLocale(), getMessages(), getTranslations("common")]);
  const saved = (await cookies()).get(THEME_COOKIE)?.value;
  const theme = saved === "light" || saved === "dark" ? saved : "system";

  return (
    <html lang={locale} data-theme={theme === "system" ? undefined : theme} suppressHydrationWarning>
      <head>
        {/* The palette, from the tokens: the one place colours are defined. */}
        <style dangerouslySetInnerHTML={{ __html: themeCss() }} />
      </head>
      <body>
        <NextIntlClientProvider locale={locale} messages={messages}>
          <ToastProvider dismissLabel={t("close")}>
            <a
              href="#main"
              className="sr-only rounded-control bg-primary px-4 py-2 text-on-primary focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50"
            >
              {t("skipToContent")}
            </a>
            <Header theme={theme} />
            <main id="main" className="mx-auto max-w-5xl px-4 py-10">
              {children}
            </main>
            <SessionWatcher />
          </ToastProvider>
        </NextIntlClientProvider>
      </body>
    </html>
  );
}

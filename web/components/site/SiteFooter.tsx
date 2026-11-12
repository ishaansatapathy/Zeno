import { getTranslations } from "next-intl/server";
import Link from "next/link";

import { Wordmark } from "./Wordmark";

export async function SiteFooter() {
  const t = await getTranslations("site");
  const year = new Date().getFullYear();
  return (
    <footer className="border-t border-white/[0.06]">
      <div className="mx-auto grid max-w-page gap-10 px-5 py-14 sm:px-8 md:grid-cols-[1.6fr_1fr_1fr]">
        <div className="flex flex-col gap-4">
          <Wordmark className="text-text" />
          <p className="max-w-xs text-sm text-text-muted">{t("footer.tagline")}</p>
        </div>
        <nav aria-label="Product" className="flex flex-col gap-2.5 text-sm">
          <a href="#how" className="text-text-muted transition-colors hover:text-text">{t("nav.how")}</a>
          <a href="#who" className="text-text-muted transition-colors hover:text-text">{t("nav.who")}</a>
          <a href="#product" className="text-text-muted transition-colors hover:text-text">{t("nav.product")}</a>
          <a href="#guarantees" className="text-text-muted transition-colors hover:text-text">{t("nav.guarantees")}</a>
        </nav>
        <nav aria-label="Account" className="flex flex-col gap-2.5 text-sm">
          <Link href="/sign-in" className="text-text-muted transition-colors hover:text-text">{t("nav.signIn")}</Link>
          <Link href="/sign-up" className="text-text-muted transition-colors hover:text-text">{t("nav.start")}</Link>
        </nav>
      </div>
      <div className="mx-auto flex max-w-page flex-wrap items-center justify-between gap-3 border-t border-white/[0.06] px-5 py-5 font-mono text-[0.72rem] text-text-muted sm:px-8">
        <span className="inline-flex items-center gap-2">
          <span className="h-1.5 w-1.5 rounded-full bg-success" />
          {t("footer.status")}
        </span>
        <span>{t("footer.made")}</span>
        <span>{t("footer.rights", { year })}</span>
      </div>
    </footer>
  );
}

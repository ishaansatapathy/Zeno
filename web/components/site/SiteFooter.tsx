import { getTranslations } from "next-intl/server";
import Link from "next/link";

import { Wordmark } from "./Wordmark";

export async function SiteFooter() {
  const t = await getTranslations("site");
  const year = new Date().getFullYear();
  return (
    <footer className="border-t border-text">
      <div className="mx-auto grid max-w-page gap-10 px-5 py-14 sm:px-8 md:grid-cols-[1.4fr_1fr_1fr]">
        <div className="flex flex-col gap-4">
          <Wordmark className="text-text" />
          <p className="max-w-xs text-sm text-text-muted">{t("footer.tagline")}</p>
        </div>
        <nav aria-label="Product" className="flex flex-col gap-2 text-sm">
          <a href="#how" className="text-text-muted hover:text-text">{t("nav.howItWorks")}</a>
          <a href="#money" className="text-text-muted hover:text-text">{t("nav.money")}</a>
          <a href="#live" className="text-text-muted hover:text-text">{t("nav.live")}</a>
          <a href="#students" className="text-text-muted hover:text-text">{t("nav.students")}</a>
        </nav>
        <nav aria-label="Account" className="flex flex-col gap-2 text-sm">
          <Link href="/sign-in" className="text-text-muted hover:text-text">{t("nav.signIn")}</Link>
          <Link href="/sign-up" className="text-text-muted hover:text-text">{t("nav.start")}</Link>
        </nav>
      </div>
      <div className="mx-auto flex max-w-page flex-wrap items-center justify-between gap-2 border-t border-border px-5 py-5 sm:px-8">
        <p className="label">{t("footer.rights", { year })}</p>
        <p className="label">{t("footer.made")}</p>
      </div>
    </footer>
  );
}

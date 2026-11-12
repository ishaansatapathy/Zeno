import { useTranslations } from "next-intl";
import Link from "next/link";

import { Wordmark } from "@/components/site/Wordmark";
import { buttonClasses } from "@/components/ui/Button";

import { ThemeToggle } from "./ThemeToggle";

export function Header({ theme }: { theme: "system" | "light" | "dark" }) {
  const t = useTranslations();
  return (
    <header className="border-b border-border">
      <div className="mx-auto flex max-w-5xl flex-wrap items-center justify-between gap-x-4 gap-y-2 px-4 py-3">
        <Link href="/" aria-label={t("common.appName")} className="text-text">
          <Wordmark />
        </Link>
        <nav aria-label="Main" className="flex flex-wrap items-center gap-2">
          <Link href="/sign-in" className={buttonClasses("ghost", "sm")}>
            {t("nav.signIn")}
          </Link>
          <Link href="/sign-up" className={buttonClasses("primary", "sm")}>
            {t("nav.signUp")}
          </Link>
          <ThemeToggle initial={theme} />
        </nav>
      </div>
    </header>
  );
}

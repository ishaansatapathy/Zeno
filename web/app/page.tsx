import { useTranslations } from "next-intl";
import Link from "next/link";

import { buttonClasses } from "@/components/ui/Button";
import { Card } from "@/components/ui/Card";

const pillars = ["secure", "pay", "live"] as const;

export default function Home() {
  const t = useTranslations("home");
  return (
    <div className="flex flex-col gap-16">
      <section className="flex max-w-3xl flex-col gap-6 py-6">
        <h1 className="text-4xl font-bold leading-tight tracking-tight sm:text-5xl">{t("title")}</h1>
        <p className="text-lg text-text-muted">{t("subtitle")}</p>
        <div className="flex flex-wrap gap-3">
          <Link href="/sign-up" className={buttonClasses("primary", "lg")}>
            {t("ctaCreator")}
          </Link>
          <Link href="/sign-in" className={buttonClasses("secondary", "lg")}>
            {t("ctaStudent")}
          </Link>
        </div>
      </section>

      <section aria-label={t("title")} className="grid gap-4 sm:grid-cols-3">
        {pillars.map((pillar) => (
          <Card key={pillar} className="flex flex-col gap-2">
            <h2 className="text-lg font-semibold">{t(`${pillar}Title`)}</h2>
            <p className="text-text-muted">{t(`${pillar}Body`)}</p>
          </Card>
        ))}
      </section>

      <footer className="border-t border-border pt-6 text-sm text-text-muted">{t("footer")}</footer>
    </div>
  );
}

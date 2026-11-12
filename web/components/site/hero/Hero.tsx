import { getTranslations } from "next-intl/server";

import { Reveal } from "../Reveal";
import { SiteButton } from "../SiteButton";

import { HeroCore } from "./HeroCore";
import { HeroHeadline } from "./HeroHeadline";

const NODES = ["record", "protect", "sell", "paid"] as const;

export async function Hero() {
  const t = await getTranslations("site.hero");
  return (
    <section id="top" className="relative overflow-hidden">
      <div className="dot-grid pointer-events-none absolute inset-0 [mask-image:radial-gradient(70%_60%_at_70%_40%,black,transparent)]" />
      <div className="relative mx-auto flex max-w-page flex-col gap-10 px-5 pb-16 pt-10 sm:px-8 lg:pb-20 lg:pt-14">
        <Reveal>
          <p className="label flex items-center gap-3">
            <span className="relative flex h-2 w-2">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-success opacity-50 motion-reduce:hidden" />
              <span className="relative inline-flex h-2 w-2 rounded-full bg-success" />
            </span>
            {t("status")}
            <span aria-hidden="true" className="h-px w-6 bg-border-strong" />
            <span className="hidden sm:inline">{t("eyebrow")}</span>
          </p>
        </Reveal>
        <HeroHeadline line1={t("line1")} line2={t("line2")} />
        <div className="grid items-center gap-12 lg:grid-cols-[1fr_1.05fr] lg:gap-10">
          <div className="flex flex-col gap-8 lg:pb-24">
            <Reveal delay={0.55} className="max-w-xl">
              <p className="text-lg leading-relaxed text-text-muted sm:text-xl">{t("body")}</p>
            </Reveal>
            <Reveal delay={0.7} className="flex flex-wrap gap-3">
              <SiteButton href="/sign-up" arrow>
                {t("primary")}
              </SiteButton>
              <SiteButton href="#how" variant="outline">
                {t("secondary")}
              </SiteButton>
            </Reveal>
          </div>
          <div className="px-12 sm:px-16 lg:-mt-24 lg:px-0">
            <HeroCore
              label={t("graphLabel")}
              nodes={NODES.map((key) => ({ key, title: t(`nodes.${key}.title`), hint: t(`nodes.${key}.hint`) }))}
            />
          </div>
        </div>
      </div>
    </section>
  );
}

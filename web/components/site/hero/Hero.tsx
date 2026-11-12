import { getTranslations } from "next-intl/server";

import { Reveal } from "../Reveal";
import { rich } from "../rich";
import { SiteButton } from "../SiteButton";
import { Tag } from "../Tag";

import { HeroCard } from "./HeroCard";

const NODES = ["upload", "transcode", "protect", "publish"] as const;

export async function Hero() {
  const t = await getTranslations("site.hero");
  return (
    <section id="top" className="relative overflow-x-clip pt-[4.5rem]">
      <div className="mx-auto flex max-w-page flex-col items-center px-5 pt-16 text-center sm:px-8 sm:pt-24">
        <Reveal>
          <Tag live>{t("tag")}</Tag>
        </Reveal>
        <h1 className="headline mt-7 max-w-6xl text-[clamp(2.6rem,6.6vw,6rem)] [text-wrap:balance]">
          <Reveal as="span" className="silver block pb-[0.06em]" delay={0.05}>
            {t("line1")}
          </Reveal>
          <Reveal as="span" className="silver block pb-[0.1em]" delay={0.15}>
            {t.rich("line2", rich)}
          </Reveal>
        </h1>
        <Reveal delay={0.3} className="mt-7 max-w-2xl">
          <p className="text-[1.08rem] leading-relaxed text-text-muted sm:text-lg">{t("body")}</p>
        </Reveal>
        <Reveal delay={0.4} className="mt-9 flex flex-wrap justify-center gap-3">
          <SiteButton href="/sign-up" arrow>
            {t("primary")}
          </SiteButton>
          <SiteButton href="#how" variant="glass">
            {t("secondary")}
          </SiteButton>
        </Reveal>
      </div>
      <div className="mx-auto mt-16 max-w-[68rem] px-5 pb-24 sm:mt-20 sm:px-8 lg:pb-32">
        <HeroCard
          title={t("card.title")}
          meta={t("card.meta")}
          status={t("card.status")}
          events={t("card.events")}
          zoom={t("card.zoom")}
          nodes={NODES.map((key) => ({ key, title: t(`nodes.${key}.title`), meta: t(`nodes.${key}.meta`) }))}
        />
      </div>
    </section>
  );
}

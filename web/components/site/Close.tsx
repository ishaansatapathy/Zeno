import { getTranslations } from "next-intl/server";

import { CloseLine } from "./CloseLine";
import { Emphasis } from "./Emphasis";
import { Guide } from "./Guide";
import { Reveal } from "./Reveal";
import { SiteButton } from "./SiteButton";

/** The end of the line: it ties itself off in a loop around the one thing left to do. */
export async function Close() {
  const t = await getTranslations("site.close");
  return (
    <section id="begin" className="relative overflow-hidden border-t border-text">
      <CloseLine />
      <div className="relative mx-auto flex max-w-page flex-col items-center gap-8 px-5 pb-28 pt-32 text-center sm:px-8 lg:pb-36 lg:pt-40">
        <Reveal>
          <h2 className="display max-w-4xl text-[clamp(2.8rem,6.6vw,6.4rem)] [text-wrap:balance]">
            {t.rich("title", { em: (chunks) => <Emphasis>{chunks}</Emphasis>, br: () => <br /> })}
          </h2>
        </Reveal>
        <Reveal delay={0.15} className="max-w-xl">
          <p className="text-lg leading-relaxed text-text-muted">{t("body")}</p>
        </Reveal>
        <Reveal delay={0.3} className="flex flex-wrap justify-center gap-3">
          <SiteButton href="/sign-up" variant="accent" arrow>
            {t("primary")}
          </SiteButton>
          <SiteButton href="/sign-in" variant="outline">
            {t("secondary")}
          </SiteButton>
        </Reveal>
        <Guide mood="smile" className="mt-6" />
      </div>
    </section>
  );
}

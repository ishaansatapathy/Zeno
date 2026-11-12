import { getTranslations } from "next-intl/server";

import { Reveal } from "./Reveal";
import { SiteButton } from "./SiteButton";

/** The last card: a polished silver plate with the one thing left to do. */
export async function FinalCta() {
  const t = await getTranslations("site.cta");
  return (
    <section id="begin" className="mx-auto max-w-page px-5 pb-24 sm:px-8 lg:pb-32">
      <Reveal y={30}>
        <div className="plate relative overflow-hidden rounded-[2rem] px-6 py-20 text-center sm:px-12 lg:py-28">
          <p className="mx-auto inline-flex items-center gap-2 rounded-full border border-black/15 px-3 py-1 font-mono text-[0.72rem] text-black/70">
            <span className="h-1.5 w-1.5 rounded-full bg-[#1f7a3d]" />
            {t("tag")}
          </p>
          <h2 className="headline mx-auto mt-7 max-w-4xl text-[clamp(2.4rem,6vw,5.4rem)] text-[#050505] [text-wrap:balance]">
            {t.rich("title", {
              i: (chunks) => <span className="font-serif text-[1.08em] font-normal italic">{chunks}</span>,
            })}
          </h2>
          <p className="mx-auto mt-6 max-w-xl text-lg leading-relaxed text-black/65">{t("body")}</p>
          <div className="mt-10 flex flex-wrap justify-center gap-3">
            <SiteButton href="/sign-up" variant="ink" arrow>
              {t("primary")}
            </SiteButton>
            <SiteButton href="/sign-in" variant="glass" className="border-black/20 bg-black/[0.04] text-[#050505] hover:bg-black/[0.08]">
              {t("secondary")}
            </SiteButton>
          </div>
        </div>
      </Reveal>
    </section>
  );
}

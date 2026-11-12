import { getTranslations } from "next-intl/server";

import { Emphasis } from "../Emphasis";
import { Guide } from "../Guide";
import { Label } from "../Label";
import { Reveal } from "../Reveal";

import { PipelineScroller, type Step } from "./PipelineScroller";

const STEPS: Step["key"][] = ["upload", "transcode", "protect", "deliver"];

export async function Pipeline() {
  const t = await getTranslations("site.pipeline");
  const steps: Step[] = STEPS.map((key) => ({ key, title: t(`steps.${key}.title`), body: t(`steps.${key}.body`) }));
  return (
    <section id="how" className="relative border-t border-text">
      <div className="mx-auto flex max-w-page flex-col gap-10 px-5 pb-16 pt-24 sm:px-8 lg:flex-row lg:items-end lg:justify-between lg:pb-10 lg:pt-32">
        <div className="flex max-w-3xl flex-col gap-6">
          <Reveal>
            <Label index="01">{t("label")}</Label>
          </Reveal>
          <Reveal delay={0.1}>
            <h2 className="display text-[clamp(2.6rem,5.6vw,5.2rem)] [text-wrap:balance]">
              {t.rich("title", { em: (chunks) => <Emphasis>{chunks}</Emphasis> })}
            </h2>
          </Reveal>
        </div>
        <Guide mood="explaining" says={t("guide")} side="right" className="self-start lg:self-end" />
      </div>
      <PipelineScroller steps={steps} />
    </section>
  );
}

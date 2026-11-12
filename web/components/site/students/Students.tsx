import { getTranslations } from "next-intl/server";

import { Emphasis } from "../Emphasis";
import { Label } from "../Label";
import { Reveal } from "../Reveal";

import { PhoneDemo } from "./PhoneDemo";

export async function Students() {
  const t = await getTranslations("site.students");
  return (
    <section id="students" className="relative overflow-hidden border-t border-text">
      <div className="dot-grid pointer-events-none absolute inset-0 [mask-image:radial-gradient(60%_60%_at_60%_60%,black,transparent)]" />
      <div className="relative mx-auto grid max-w-page items-center gap-16 px-5 py-24 sm:px-8 lg:grid-cols-[1fr_1.2fr] lg:py-32">
        <div className="flex flex-col gap-6">
          <Reveal>
            <Label index="04">{t("label")}</Label>
          </Reveal>
          <Reveal delay={0.1}>
            <h2 className="display text-[clamp(2.6rem,5.6vw,5.2rem)] [text-wrap:balance]">
              {t.rich("title", { em: (chunks) => <Emphasis>{chunks}</Emphasis> })}
            </h2>
          </Reveal>
          <Reveal delay={0.2} className="max-w-md">
            <p className="text-lg leading-relaxed text-text-muted">{t("body")}</p>
          </Reveal>
        </div>
        <Reveal y={30}>
          <PhoneDemo />
        </Reveal>
      </div>
    </section>
  );
}

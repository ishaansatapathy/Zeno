import { getTranslations } from "next-intl/server";

import { Reveal } from "../Reveal";
import { rich } from "../rich";
import { Tag } from "../Tag";

import { StackCards, type Guarantee } from "./StackCards";

export async function Guarantees() {
  const t = await getTranslations("site.stack");
  return (
    <section id="guarantees" className="relative pb-24">
      <div className="mx-auto flex max-w-page flex-col gap-6 px-5 pb-10 pt-24 sm:px-8 lg:pt-32">
        <Reveal>
          <Tag>{t("tag")}</Tag>
        </Reveal>
        <Reveal delay={0.1}>
          <h2 className="headline silver text-[clamp(2.4rem,5vw,4.4rem)]">{t.rich("title", rich)}</h2>
        </Reveal>
      </div>
      <StackCards items={t.raw("cards") as Guarantee[]} />
    </section>
  );
}

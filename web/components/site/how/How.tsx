import { getTranslations } from "next-intl/server";

import { Reveal } from "../Reveal";
import { rich } from "../rich";
import { Tag } from "../Tag";

import { Colonnade, type Column } from "./Colonnade";

const KEYS: Column["key"][] = ["upload", "transcode", "protect", "deliver"];

export async function How() {
  const t = await getTranslations("site.how");
  const columns: Column[] = KEYS.map((key) => ({
    key,
    label: t(`columns.${key}.label`),
    title: t(`columns.${key}.title`),
    body: t(`columns.${key}.body`),
  }));
  return (
    <section id="how" className="relative">
      <div className="mx-auto grid max-w-page gap-6 px-5 pb-12 pt-24 sm:px-8 lg:grid-cols-[1.3fr_1fr] lg:items-end lg:pt-32">
        <div className="flex flex-col gap-6">
          <Reveal>
            <Tag>{t("tag")}</Tag>
          </Reveal>
          <Reveal delay={0.1}>
            <h2 className="headline silver text-[clamp(2.4rem,5vw,4.4rem)] [text-wrap:balance]">{t.rich("title", rich)}</h2>
          </Reveal>
        </div>
        <Reveal delay={0.2} className="max-w-md lg:justify-self-end">
          <p className="text-lg leading-relaxed text-text-muted">{t("body")}</p>
        </Reveal>
      </div>
      <Colonnade columns={columns} />
    </section>
  );
}

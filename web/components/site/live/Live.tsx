import { createAvatar } from "@dicebear/core";
import { openPeeps } from "@dicebear/collection";
import { getTranslations } from "next-intl/server";

import { Emphasis } from "../Emphasis";
import { Label } from "../Label";
import { Reveal } from "../Reveal";

import { LiveDemo } from "./LiveDemo";

/** Classmates' faces: Open Peeps (CC0), one per name, drawn on the server. */
function face(seed: string): string {
  return createAvatar(openPeeps, {
    seed,
    face: ["smile", "calm", "cute", "smileBig", "awe"],
    accessoriesProbability: 25,
    facialHairProbability: 15,
    maskProbability: 0,
  }).toString();
}

export async function Live() {
  const t = await getTranslations("site.live");
  const chat = (t.raw("chat") as { name: string; text: string }[]).map((m) => ({ ...m, avatar: face(m.name) }));
  return (
    <section id="live" className="relative border-t border-text">
      <div className="mx-auto flex max-w-page flex-col gap-14 px-5 py-24 sm:px-8 lg:py-32">
        <div className="grid gap-8 lg:grid-cols-[1.1fr_1fr] lg:items-end">
          <div className="flex flex-col gap-6">
            <Reveal>
              <Label index="03">{t("label")}</Label>
            </Reveal>
            <Reveal delay={0.1}>
              <h2 className="display text-[clamp(2.6rem,5.6vw,5.2rem)] [text-wrap:balance]">
                {t.rich("title", { em: (chunks) => <Emphasis>{chunks}</Emphasis> })}
              </h2>
            </Reveal>
          </div>
          <Reveal delay={0.2} className="max-w-lg lg:justify-self-end">
            <p className="text-lg leading-relaxed text-text-muted">{t("body")}</p>
          </Reveal>
        </div>
        <Reveal y={30}>
          <LiveDemo chat={chat} lessonsBefore={t.raw("lessons") as string[]} />
        </Reveal>
      </div>
    </section>
  );
}

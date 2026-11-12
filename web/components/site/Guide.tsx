import { getTranslations } from "next-intl/server";

import { guideSvg, type Mood } from "@/lib/site/guide";

import { GuideFigure } from "./GuideFigure";

/** The guide, with something to say. Drawn on the server; only the blink and the bob are client-side. */
export async function Guide({
  mood,
  says,
  side = "left",
  className,
}: {
  mood: Mood;
  says?: string;
  side?: "left" | "right";
  className?: string;
}) {
  const t = await getTranslations("site.guide");
  return (
    <GuideFigure
      face={guideSvg(mood)}
      blink={guideSvg("eyesClosed")}
      says={says}
      side={side}
      alt={t("alt")}
      className={className}
    />
  );
}

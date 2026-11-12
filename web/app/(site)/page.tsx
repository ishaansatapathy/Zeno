import { getTranslations } from "next-intl/server";

import { FinalCta } from "@/components/site/FinalCta";
import { Hero } from "@/components/site/hero/Hero";
import { How } from "@/components/site/how/How";
import { Guarantees } from "@/components/site/stack/Guarantees";
import { Statement } from "@/components/site/statement/Statement";
import { Track } from "@/components/site/track/Track";
import { Who } from "@/components/site/who/Who";

/**
 * The home page: what Zeno is, how a lesson travels, who it is for, the product itself, the
 * guarantees underneath, and the one thing to do next.
 */
export default async function Home() {
  const t = await getTranslations("site.statement");
  return (
    <>
      <Hero />
      <How />
      <Statement lines={[t("line1"), t("line2"), t("line3"), t("line4")]} chips={t.raw("chips") as string[]} />
      <Who />
      <Track />
      <Guarantees />
      <FinalCta />
    </>
  );
}

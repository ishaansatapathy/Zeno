import { getTranslations } from "next-intl/server";

import { Emphasis } from "../Emphasis";
import { Guide } from "../Guide";
import { Label } from "../Label";
import { Reveal } from "../Reveal";

import { LedgerChain } from "./LedgerChain";
import { ReceiptDemo } from "./ReceiptDemo";

export async function Money() {
  const t = await getTranslations("site.money");
  return (
    <section id="money" className="relative border-t border-text bg-surface-strong/40">
      <div className="mx-auto grid max-w-page gap-16 px-5 py-24 sm:px-8 lg:grid-cols-[1fr_1.05fr] lg:gap-20 lg:py-32">
        <div className="flex flex-col gap-8">
          <Reveal>
            <Label index="02">{t("label")}</Label>
          </Reveal>
          <Reveal delay={0.1}>
            <h2 className="display text-[clamp(2.6rem,5.6vw,5.2rem)] [text-wrap:balance]">
              {t.rich("title", { em: (chunks) => <Emphasis>{chunks}</Emphasis> })}
            </h2>
          </Reveal>
          <Reveal delay={0.2} className="max-w-lg">
            <p className="text-lg leading-relaxed text-text-muted">{t("body")}</p>
          </Reveal>
          <LedgerChain steps={[t("chain.checkout"), t("chain.confirmed"), t("chain.ledger"), t("chain.payout")]} />
          <Reveal delay={0.1} className="max-w-md">
            <p className="text-sm leading-relaxed text-text-muted">{t("methods")}</p>
          </Reveal>
          <Guide mood="smileBig" says={t("guide")} className="mt-2" />
        </div>
        <Reveal y={30}>
          <ReceiptDemo />
        </Reveal>
      </div>
    </section>
  );
}

import { getTranslations } from "next-intl/server";

import { rich } from "../rich";
import { Tag } from "../Tag";

import { CardTrack } from "./CardTrack";
import { LedgerCard, LiveCard, ReceiptCard, StudentsCard } from "./cards";

export async function Track() {
  const t = await getTranslations("site.track");
  const header = (
    <div className="flex flex-col gap-5">
      <Tag>{t("tag")}</Tag>
      <h2 className="headline silver max-w-3xl text-[clamp(2.4rem,5vw,4.4rem)] [text-wrap:balance]">{t.rich("title", rich)}</h2>
    </div>
  );
  const cards = [
    { key: "payments", body: <ReceiptCard /> },
    { key: "live", body: <LiveCard /> },
    { key: "students", body: <StudentsCard /> },
    { key: "ledger", body: <LedgerCard /> },
  ].map((card) => ({ ...card, label: t(`cards.${card.key}.label`), title: t(`cards.${card.key}.title`) }));
  return (
    <section id="product" className="relative">
      <CardTrack cards={cards} header={header} />
    </section>
  );
}

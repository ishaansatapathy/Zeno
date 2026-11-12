import { getTranslations } from "next-intl/server";

import { feeFor, formatMoney } from "@/lib/site/receipt";

import { Reveal } from "../Reveal";
import { rich } from "../rich";
import { Tag } from "../Tag";

import { SegmentStack, type Segment } from "./SegmentStack";

const PRICE = 4_900; // $49
const SALES = 38;
const REFUNDS = 1;

const SEGMENTS: { key: string; href: string }[] = [
  { key: "creators", href: "/sign-up" },
  { key: "cohorts", href: "/sign-up" },
  { key: "students", href: "/sign-in" },
];

export async function Who() {
  const t = await getTranslations("site.who");
  const segments: Segment[] = SEGMENTS.map(({ key, href }) => ({
    key,
    href,
    label: t(`segments.${key}.label`),
    line: t(`segments.${key}.line`),
    points: t.raw(`segments.${key}.points`) as string[],
    cta: t(`segments.${key}.cta`),
  }));
  // The widget's sum is the ledger's: every sale pays its fee, a refund returns the price.
  const net = SALES * PRICE - SALES * feeFor(PRICE, 290) - REFUNDS * PRICE;

  return (
    <section id="who" className="relative">
      <div className="mx-auto grid max-w-page gap-14 px-5 py-24 sm:px-8 lg:grid-cols-[0.9fr_1.1fr] lg:gap-16 lg:py-32">
        <div className="flex flex-col gap-6">
          <Reveal>
            <Tag>{t("tag")}</Tag>
          </Reveal>
          <Reveal delay={0.1}>
            <h2 className="headline silver text-[clamp(2.4rem,5vw,4.4rem)] [text-wrap:balance]">{t.rich("title", rich)}</h2>
          </Reveal>
          <Reveal delay={0.2} className="max-w-md">
            <p className="text-lg leading-relaxed text-text-muted">{t("body")}</p>
          </Reveal>
          <Reveal delay={0.3} className="mt-auto">
            <div className="metal max-w-sm p-5">
              <div className="flex items-center justify-between font-mono text-[0.72rem] text-text-muted">
                <span>{t("widget.title")}</span>
                <span className="inline-flex items-center gap-1.5 text-success">
                  <span className="h-1.5 w-1.5 rounded-full bg-success" />
                  {t("widget.status")}
                </span>
              </div>
              <dl className="mt-5 flex flex-col gap-3 font-mono text-[0.78rem]">
                <div className="grid grid-cols-[1fr_auto] gap-y-1.5 text-text-muted">
                  <dt>{t("widget.sales")}</dt>
                  <dd className="tabular-nums text-text">{SALES}</dd>
                  <dd aria-hidden="true" className="col-span-2 h-1.5 rounded-full bg-white/[0.06]">
                    <span className="block h-full w-[76%] rounded-full bg-[linear-gradient(90deg,#a1a1aa,#f4f4f5)]" />
                  </dd>
                </div>
                <div className="grid grid-cols-[1fr_auto] gap-y-1.5 text-text-muted">
                  <dt>{t("widget.refunds")}</dt>
                  <dd className="tabular-nums text-text">{REFUNDS}</dd>
                  <dd aria-hidden="true" className="col-span-2 h-1.5 rounded-full bg-white/[0.06]">
                    <span className="block h-full w-[4%] rounded-full bg-white/40" />
                  </dd>
                </div>
              </dl>
              <p className="mt-5 flex items-baseline justify-between border-t border-white/[0.07] pt-4">
                <span className="font-mono text-[0.72rem] text-text-muted">{t("widget.net")}</span>
                <span className="headline silver text-[1.9rem] tabular-nums">{formatMoney(net, "USD")}</span>
              </p>
            </div>
          </Reveal>
        </div>
        <Reveal y={24}>
          <SegmentStack segments={segments} />
        </Reveal>
      </div>
    </section>
  );
}

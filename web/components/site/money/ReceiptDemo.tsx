"use client";

import { m, useSpring, useTransform } from "motion/react";
import { useTranslations } from "next-intl";
import { useEffect, useId, useState } from "react";

import { cn } from "@/components/ui/cn";
import {
  EXAMPLE_FEE_BPS,
  REFUND_WINDOW_DAYS,
  currencies,
  currencyInfo,
  formatMoney,
  formatRate,
  receipt,
  type Currency,
} from "@/lib/site/receipt";

/** A number that rolls to its new value instead of jumping, formatted as money. */
function Rolling({ value, currency, className }: { value: number; currency: Currency; className?: string }) {
  const spring = useSpring(value, { stiffness: 90, damping: 20, mass: 0.6 });
  useEffect(() => spring.set(value), [spring, value]);
  const text = useTransform(spring, (v) => formatMoney(Math.round(v), currency));
  return <m.span className={cn("tabular-nums", className)}>{text}</m.span>;
}

/**
 * The receipt: pick a currency, a price and how many students, and see the whole sum. It is the
 * same arithmetic as the ledger (fee per sale, rounded half up), so the example is honest.
 */
export function ReceiptDemo() {
  const t = useTranslations("site.money");
  const ids = useId();
  const [currency, setCurrency] = useState<Currency>("USD");
  const [priceIndex, setPriceIndex] = useState(1);
  const [sales, setSales] = useState(120);
  const price = currencyInfo[currency].prices[priceIndex];
  const r = receipt(price, sales);

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-4 rounded-card border border-text bg-surface p-5">
        <fieldset className="flex flex-wrap items-center justify-between gap-3">
          <legend className="label float-left mr-3 py-2">{t("currency")}</legend>
          <div className="flex rounded-full border border-text p-1">
            {currencies.map((c) => (
              <label key={c} className="cursor-pointer">
                <input type="radio" name={`${ids}-currency`} value={c} checked={c === currency} onChange={() => setCurrency(c)} className="peer sr-only" />
                <span className="block rounded-full px-3 py-1 font-mono text-xs transition-colors peer-checked:bg-text peer-checked:text-bg peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-focus">
                  {c}
                </span>
              </label>
            ))}
          </div>
        </fieldset>
        <fieldset className="flex flex-wrap items-center justify-between gap-3">
          <legend className="label float-left mr-3 py-2">{t("price")}</legend>
          <div className="flex flex-wrap gap-2">
            {currencyInfo[currency].prices.map((amount, i) => (
              <label key={amount} className="cursor-pointer">
                <input type="radio" name={`${ids}-price`} checked={i === priceIndex} onChange={() => setPriceIndex(i)} className="peer sr-only" />
                <span className="block rounded-full border border-text px-3 py-1 text-sm tabular-nums transition-colors peer-checked:bg-accent peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-focus">
                  {formatMoney(amount, currency)}
                </span>
              </label>
            ))}
          </div>
        </fieldset>
        <div className="flex items-center gap-4">
          <label htmlFor={`${ids}-sales`} className="label shrink-0">
            {t("students")}
          </label>
          <input
            id={`${ids}-sales`}
            type="range"
            min={1}
            max={500}
            value={sales}
            onChange={(e) => setSales(Number(e.target.value))}
            className="h-2 w-full cursor-pointer appearance-none rounded-full bg-surface-strong accent-[rgb(var(--text))]"
          />
          <output htmlFor={`${ids}-sales`} className="w-12 text-right font-mono text-sm tabular-nums">
            {sales}
          </output>
        </div>
      </div>

      <div className="relative">
      <div
        className="relative rounded-t-card border border-b-0 border-text bg-surface px-6 pb-10 pt-6 shadow-[8px_8px_0_0_rgb(var(--text))] [mask-image:radial-gradient(circle_at_10px_100%,transparent_7px,black_7.5px)] [mask-position:-10px_0] [mask-size:20px_100%] [mask-repeat:repeat-x]"
        aria-live="polite"
      >
        <div className="flex items-center justify-between border-b border-dashed border-border-strong pb-3">
          <p className="label text-text">{t("receipt.title")}</p>
          <p className="label">Zeno</p>
        </div>
        <dl className="flex flex-col gap-3 py-4 font-mono text-sm">
          <div className="flex justify-between gap-4">
            <dt>{t("receipt.sales", { count: sales, price: formatMoney(price, currency) })}</dt>
            <dd>
              <Rolling value={r.gross} currency={currency} />
            </dd>
          </div>
          <div className="flex justify-between gap-4 text-text-muted">
            <dt>{t("receipt.fee", { rate: formatRate(EXAMPLE_FEE_BPS) })}</dt>
            <dd>
              −<Rolling value={r.fees} currency={currency} />
            </dd>
          </div>
          <div className="flex justify-between gap-4 text-text-muted">
            <dt>{t("receipt.window")}</dt>
            <dd>{t("receipt.windowValue", { days: REFUND_WINDOW_DAYS })}</dd>
          </div>
        </dl>
        <div className="flex items-end justify-between gap-4 border-t border-dashed border-border-strong pt-4">
          <p className="label text-text">{t("receipt.net")}</p>
          <p className="font-serif text-[clamp(2.4rem,4.4vw,3.6rem)] leading-none">
            <Rolling value={r.net} currency={currency} />
          </p>
        </div>
      </div>
        <m.p
          className="absolute -right-3 -top-4 z-10 rotate-[8deg] rounded-full border-2 border-success bg-surface px-3 py-1 font-mono text-[0.68rem] uppercase tracking-label text-success"
          initial={{ opacity: 0, scale: 1.6 }}
          whileInView={{ opacity: 1, scale: 1 }}
          viewport={{ once: true, margin: "0px 0px -20% 0px" }}
          transition={{ type: "spring", stiffness: 320, damping: 14, delay: 0.6 }}
        >
          {t("receipt.paid")}
        </m.p>
      </div>
      <p className="text-xs leading-relaxed text-text-muted">{t("receipt.note")}</p>
    </div>
  );
}

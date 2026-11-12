"use client";

import { useTranslations } from "next-intl";
import { useId, useState } from "react";

import { cn } from "@/components/ui/cn";
import { EXAMPLE_FEE_BPS, REFUND_WINDOW_DAYS, formatMoney, formatRate, receipt } from "@/lib/site/receipt";

/** Payments: the receipt for a month of sales, worked out the way the ledger works it out. */
export function ReceiptCard() {
  const t = useTranslations("site.track.receipt");
  const price = 4_900;
  const sales = 120;
  const r = receipt(price, sales);
  return (
    <div className="flex h-full flex-col justify-end gap-3 font-mono text-[0.82rem]">
      <div className="flex justify-between gap-4">
        <span className="text-text-muted">{t("sales", { count: sales, price: formatMoney(price, "USD") })}</span>
        <span className="tabular-nums">{formatMoney(r.gross, "USD")}</span>
      </div>
      <div className="flex justify-between gap-4 text-text-muted">
        <span>{t("fee", { rate: formatRate(EXAMPLE_FEE_BPS) })}</span>
        <span className="tabular-nums">−{formatMoney(r.fees, "USD")}</span>
      </div>
      <div className="flex justify-between gap-4 text-text-muted">
        <span>{t("window")}</span>
        <span>{t("windowValue", { days: REFUND_WINDOW_DAYS })}</span>
      </div>
      <div className="mt-2 flex items-end justify-between border-t border-dashed border-white/15 pt-4">
        <span className="text-text-muted">{t("net")}</span>
        <span className="headline silver font-sans text-[2.6rem] tabular-nums">{formatMoney(r.net, "USD")}</span>
      </div>
    </div>
  );
}

/** Live: a class in progress, its chat, and the line that says it will become a lesson. */
export function LiveCard() {
  const t = useTranslations("site.track.live");
  const chat = t.raw("chat") as { name: string; text: string }[];
  return (
    <div className="flex h-full flex-col gap-3">
      <div className="relative flex-1 overflow-hidden rounded-xl border border-white/10 bg-[radial-gradient(90%_80%_at_30%_20%,rgb(255_255_255/0.08),transparent_60%),#0a0a0b]">
        <div className="absolute left-3 top-3 flex gap-2 font-mono text-[0.68rem]">
          <span className="inline-flex items-center gap-1.5 rounded-full bg-danger px-2 py-0.5 text-on-danger">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-on-danger" />
            {t("badge")}
          </span>
          <span className="rounded-full border border-white/15 px-2 py-0.5 text-text-muted">{t("watching", { count: 126 })}</span>
        </div>
      </div>
      <ul className="flex flex-col gap-2 text-[0.85rem]">
        {chat.map((message) => (
          <li key={message.name} className="flex gap-2">
            <span className="text-text">{message.name}</span>
            <span className="truncate text-text-muted">{message.text}</span>
          </li>
        ))}
      </ul>
      <p className="font-mono text-[0.72rem] text-success">→ {t("saved")}</p>
    </div>
  );
}

const NETS = [
  { key: "slow", quality: "360p", blur: 2.4 },
  { key: "mobile", quality: "480p", blur: 1 },
  { key: "wifi", quality: "1080p", blur: 0 },
] as const;

/** Students: pick a connection and watch the picture adapt. */
export function StudentsCard() {
  const t = useTranslations("site.track.students");
  const ids = useId();
  const [net, setNet] = useState<(typeof NETS)[number]>(NETS[1]);
  return (
    <div className="flex h-full flex-col gap-4">
      <div className="relative flex-1 overflow-hidden rounded-xl border border-white/10">
        <div
          aria-hidden="true"
          className="absolute inset-0 bg-[linear-gradient(135deg,#3f3f46,#18181b_45%,#52525b)] transition-[filter] duration-500"
          style={{ filter: `blur(${net.blur}px)` }}
        >
          <svg viewBox="0 0 160 90" className="h-full w-full">
            <path d="M44 74 C 40 46, 64 24, 98 22 C 100 52, 80 74, 44 74 Z" fill="none" stroke="#e4e4e7" strokeWidth="1.4" />
            <path d="M44 74 C 60 58, 74 44, 96 24" fill="none" stroke="#e4e4e7" strokeWidth="1" />
            <circle cx="128" cy="22" r="10" fill="none" stroke="#e4e4e7" strokeWidth="1.2" />
          </svg>
        </div>
        <span className="absolute right-2 top-2 rounded-full border border-white/20 bg-black/60 px-2 py-0.5 font-mono text-[0.66rem]">
          {t("quality", { quality: net.quality })}
        </span>
        <span className="absolute inset-x-2 bottom-2 h-1 overflow-hidden rounded-full bg-white/15">
          <span className="block h-full w-[42%] rounded-full bg-[#e4e4e7]" />
        </span>
      </div>
      <p className="text-[0.9rem]">{t("lesson")}</p>
      <fieldset className="flex gap-1.5 rounded-full border border-white/10 p-1">
        <legend className="sr-only">{t("lesson")}</legend>
        {NETS.map((option) => (
          <label key={option.key} className="flex-1 cursor-pointer">
            <input type="radio" name={`${ids}-net`} checked={net.key === option.key} onChange={() => setNet(option)} className="peer sr-only" />
            <span className="block rounded-full py-1.5 text-center text-[0.8rem] text-text-muted transition-colors peer-checked:bg-white/90 peer-checked:text-black peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-focus">
              {t(`nets.${option.key}`)}
            </span>
          </label>
        ))}
      </fieldset>
    </div>
  );
}

/** Ledger: the entries for one sale, and the balance that has to come out at zero. */
export function LedgerCard() {
  const t = useTranslations("site.track.ledger");
  const rows = t.raw("rows") as { label: string; amount: string }[];
  return (
    <div className="flex h-full flex-col justify-end gap-2 font-mono text-[0.82rem]">
      {rows.map((row, i) => (
        <div key={row.label} className={cn("flex justify-between gap-4 border-b border-white/[0.06] py-2", i === 0 ? "text-text" : "text-text-muted")}>
          <span>{row.label}</span>
          <span className="tabular-nums">{row.amount}</span>
        </div>
      ))}
      <div className="mt-2 flex items-center justify-between">
        <span className="text-text-muted">{t("balance")}</span>
        <span className="inline-flex items-center gap-2 text-success">
          <svg viewBox="0 0 12 12" aria-hidden="true" className="h-3 w-3">
            <path d="M2 6.5 5 9.5 10 3" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          0.00 · {t("balanced")}
        </span>
      </div>
    </div>
  );
}

import type { Locale } from "./locale";

/**
 * Money, dates and numbers, always through `Intl`, never by hand and never by gluing strings
 * together. Everything is explicit about the time zone and the currency, because "whatever this
 * machine thinks" is how a server and a browser end up rendering different text (a hydration
 * mismatch) and how a creator in Pune sees yesterday's date.
 */

export const TIME_ZONE = "Asia/Kolkata";

/** English as it is written in India: lakhs and crores, DD Mon YYYY. */
const intlLocale: Record<Locale, string> = { en: "en-IN" };

/** An amount in paise (the API's unit everywhere) as rupees: 499900 -> "₹4,999". */
export function formatRupees(paise: number, locale: Locale): string {
  if (!Number.isSafeInteger(paise)) throw new RangeError("an amount is a whole number of paise");
  const whole = paise % 100 === 0;
  return new Intl.NumberFormat(intlLocale[locale], {
    style: "currency",
    currency: "INR",
    minimumFractionDigits: whole ? 0 : 2,
    maximumFractionDigits: 2,
  }).format(paise / 100);
}

/** A moment, as the date and time it is in India. */
export function formatDateTime(moment: string | Date, locale: Locale): string {
  return new Intl.DateTimeFormat(intlLocale[locale], {
    timeZone: TIME_ZONE,
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(moment));
}

/** A moment, as the date it is in India. */
export function formatDate(moment: string | Date, locale: Locale): string {
  return new Intl.DateTimeFormat(intlLocale[locale], {
    timeZone: TIME_ZONE,
    dateStyle: "medium",
  }).format(new Date(moment));
}

export function formatNumber(value: number, locale: Locale): string {
  return new Intl.NumberFormat(intlLocale[locale]).format(value);
}

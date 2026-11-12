/**
 * The arithmetic behind the receipt on the home page. Amounts are integers in the currency's minor
 * unit (cents, paise), as they are everywhere in Zeno; the fee is worked out per sale and rounded
 * half up, the way the ledger does it, so the example never disagrees with the product.
 */

export const currencies = ["USD", "EUR", "GBP", "INR"] as const;
export type Currency = (typeof currencies)[number];

/** How each currency is written, and a few sensible course prices in it (minor units). */
export const currencyInfo: Record<Currency, { locale: string; prices: readonly number[] }> = {
  USD: { locale: "en-US", prices: [1_900, 4_900, 9_900, 19_900] },
  EUR: { locale: "en-IE", prices: [1_900, 4_900, 9_900, 19_900] },
  GBP: { locale: "en-GB", prices: [1_500, 3_900, 7_900, 14_900] },
  INR: { locale: "en-IN", prices: [49_900, 99_900, 2_49_900, 4_99_900] },
};

/** A typical card rate, used only as an example: 2.9%. */
export const EXAMPLE_FEE_BPS = 290;
export const REFUND_WINDOW_DAYS = 7;

export type Receipt = { gross: number; fees: number; net: number; feePerSale: number };

/** The fee on one sale: basis points of the price, rounded half up to a whole minor unit. */
export function feeFor(price: number, bps: number): number {
  if (!Number.isSafeInteger(price) || price < 0) throw new RangeError("a price is a whole, non-negative amount");
  if (!Number.isInteger(bps) || bps < 0 || bps > 10_000) throw new RangeError("a rate is 0 to 10,000 basis points");
  return Math.floor((price * bps + 5_000) / 10_000);
}

export function receipt(price: number, sales: number, bps = EXAMPLE_FEE_BPS): Receipt {
  if (!Number.isInteger(sales) || sales < 0) throw new RangeError("sales are a whole, non-negative count");
  const feePerSale = feeFor(price, bps);
  const gross = price * sales;
  const fees = feePerSale * sales;
  if (!Number.isSafeInteger(gross)) throw new RangeError("too large to add up exactly");
  return { gross, fees, net: gross - fees, feePerSale };
}

/** Money as people read it in that currency: "$4,900.00", "₹4,99,900". */
export function formatMoney(minor: number, currency: Currency): string {
  const whole = minor % 100 === 0;
  return new Intl.NumberFormat(currencyInfo[currency].locale, {
    style: "currency",
    currency,
    minimumFractionDigits: whole ? 0 : 2,
    maximumFractionDigits: 2,
  }).format(minor / 100);
}

/** "2.9%" from 290 basis points. */
export function formatRate(bps: number): string {
  return `${(bps / 100).toLocaleString("en-US", { maximumFractionDigits: 2 })}%`;
}

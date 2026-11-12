import { describe, expect, it } from "vitest";

import { currencies, currencyInfo, feeFor, formatMoney, formatRate, receipt } from "./receipt";

describe("the fee on one sale", () => {
  it("is basis points of the price, rounded half up", () => {
    expect(feeFor(4_900, 290)).toBe(142); // 142.1
    expect(feeFor(1_900, 290)).toBe(55); // 55.1
    expect(feeFor(5_000, 290)).toBe(145);
    expect(feeFor(50, 100)).toBe(1); // 0.5 rounds up
    expect(feeFor(49, 100)).toBe(0); // 0.49 rounds down
    expect(feeFor(0, 290)).toBe(0);
  });

  it("refuses what is not money or not a rate", () => {
    expect(() => feeFor(10.5, 290)).toThrow(RangeError);
    expect(() => feeFor(-1, 290)).toThrow(RangeError);
    expect(() => feeFor(100, -1)).toThrow(RangeError);
    expect(() => feeFor(100, 10_001)).toThrow(RangeError);
    expect(() => feeFor(100, 2.5)).toThrow(RangeError);
  });
});

describe("a receipt", () => {
  it("charges the fee per sale, as the ledger does, so it never disagrees with the product", () => {
    // 120 sales of $49: $5,880 in, $170.40 in fees (142 cents each), $5,709.60 out.
    expect(receipt(4_900, 120)).toEqual({ gross: 588_000, fees: 17_040, net: 570_960, feePerSale: 142 });
  });

  it("always adds up: what came in is what went to fees plus what is yours", () => {
    for (const currency of currencies) {
      for (const price of currencyInfo[currency].prices) {
        for (const sales of [0, 1, 7, 120, 500]) {
          const r = receipt(price, sales);
          expect(r.fees + r.net, `${currency} ${price} x ${sales}`).toBe(r.gross);
          expect(r.net).toBeGreaterThanOrEqual(0);
        }
      }
    }
  });

  it("is empty with no sales, and refuses a nonsense count", () => {
    expect(receipt(4_900, 0)).toEqual({ gross: 0, fees: 0, net: 0, feePerSale: 142 });
    expect(() => receipt(4_900, -1)).toThrow(RangeError);
    expect(() => receipt(4_900, 1.5)).toThrow(RangeError);
    expect(() => receipt(Number.MAX_SAFE_INTEGER, 2)).toThrow(RangeError);
  });
});

describe("money and rates, as people read them", () => {
  it("writes each currency its own way, without cents when there are none", () => {
    expect(formatMoney(4_900, "USD")).toBe("$49");
    expect(formatMoney(570_960, "USD")).toBe("$5,709.60");
    expect(formatMoney(4_99_900, "INR")).toBe("₹4,999");
    expect(formatMoney(1_23_45_600, "INR")).toBe("₹1,23,456");
    expect(formatMoney(3_900, "GBP")).toBe("£39");
    expect(formatMoney(4_950, "EUR")).toBe("€49.50");
  });

  it("writes a rate as a percentage", () => {
    expect(formatRate(290)).toBe("2.9%");
    expect(formatRate(236)).toBe("2.36%");
    expect(formatRate(300)).toBe("3%");
  });
});

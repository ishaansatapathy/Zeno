// Screenshots of the public site at desktop and phone sizes, for looking at the design.
//
//   node scripts/shoot.mjs <out-dir> [url] [--full] [--scroll=0,900,1800]
//
// Waits for fonts and the first animations to settle. With --scroll it scrolls step by step
// (through the page's own smooth scroll) and takes one viewport shot per stop.

import { chromium } from "@playwright/test";

const args = process.argv.slice(2);
const out = args[0] ?? ".";
const url = args.find((a) => a.startsWith("http")) ?? "http://localhost:3000/";
const full = args.includes("--full");
const scrollArg = args.find((a) => a.startsWith("--scroll="));
const stops = scrollArg ? scrollArg.slice(9).split(",").map(Number) : null;
const only = args.find((a) => a.startsWith("--only="))?.slice(7);

const sizes = [
  { name: "desktop", width: 1440, height: 900, isMobile: false },
  { name: "phone", width: 390, height: 844, isMobile: true },
].filter((s) => !only || s.name === only);

const browser = await chromium.launch();
for (const size of sizes) {
  const page = await browser.newPage({
    viewport: { width: size.width, height: size.height },
    deviceScaleFactor: 1,
    isMobile: size.isMobile,
    hasTouch: size.isMobile,
  });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.goto(url, { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(2600);
  if (stops) {
    for (const y of stops) {
      await page.mouse.wheel(0, 0);
      await page.evaluate((top) => window.scrollTo({ top, behavior: "instant" }), y);
      await page.waitForTimeout(1400);
      await page.screenshot({ path: `${out}/${size.name}-${y}.png` });
    }
  } else {
    await page.screenshot({ path: `${out}/${size.name}.png`, fullPage: full });
  }
  console.log(size.name, "height", await page.evaluate(() => document.documentElement.scrollHeight), errors.length ? `ERRORS: ${errors.join(" | ").slice(0, 600)}` : "no errors");
  await page.close();
}
await browser.close();

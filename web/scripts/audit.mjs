// Accessibility and loading checks for a page, at desktop and phone sizes.
//
//   node scripts/audit.mjs [url]
//
// Scrolls the whole page first (so everything that reveals on scroll is in its final state), then
// runs axe-core with the WCAG 2.1 A and AA rules, and reports the largest contentful paint and the
// layout shift measured during the load. Exits non-zero on any violation.

import { createRequire } from "node:module";

import { chromium } from "@playwright/test";

const require = createRequire(import.meta.url);
const url = process.argv.find((a) => a.startsWith("http")) ?? "http://localhost:3000/";
const sizes = [
  { name: "desktop", width: 1440, height: 900, isMobile: false },
  { name: "phone", width: 390, height: 844, isMobile: true },
];

const browser = await chromium.launch();
let failures = 0;
for (const size of sizes) {
  const page = await browser.newPage({ viewport: { width: size.width, height: size.height }, isMobile: size.isMobile, hasTouch: size.isMobile });
  await page.addInitScript(() => {
    window.__vitals = { lcp: 0, cls: 0 };
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) window.__vitals.lcp = entry.startTime;
    }).observe({ type: "largest-contentful-paint", buffered: true });
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) if (!entry.hadRecentInput) window.__vitals.cls += entry.value;
    }).observe({ type: "layout-shift", buffered: true });
  });
  await page.goto(url, { waitUntil: "networkidle" });
  await page.waitForTimeout(1500);
  const vitals = await page.evaluate(() => window.__vitals);

  const height = await page.evaluate(() => document.documentElement.scrollHeight);
  for (let y = 0; y < height; y += size.height / 2) {
    await page.evaluate((top) => window.scrollTo({ top, behavior: "instant" }), y);
    await page.waitForTimeout(250);
  }
  await page.waitForTimeout(1500);

  await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
  const result = await page.evaluate(() =>
    window.axe.run(document, { runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"] } }),
  );
  const serious = result.violations;
  failures += serious.length;
  console.log(`\n${size.name}: LCP ${Math.round(vitals.lcp)} ms, CLS ${vitals.cls.toFixed(3)}, ${serious.length} violation(s), ${result.passes.length} rules passed`);
  for (const v of serious) {
    console.log(`  [${v.impact}] ${v.id}: ${v.help} (${v.nodes.length})`);
    for (const node of v.nodes.slice(0, 3)) console.log(`      ${node.target.join(" ")} :: ${node.failureSummary?.split("\n")[1]?.trim() ?? ""}`);
  }
  await page.close();
}
await browser.close();
process.exit(failures ? 1 : 0);

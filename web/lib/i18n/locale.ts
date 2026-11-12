/**
 * The languages the app is written in. Every string already lives in `messages/<language>.json`,
 * so adding a language is a file and a line here, not a rewrite. For now there is one.
 */
export const locales = ["en"] as const;
export type Locale = (typeof locales)[number];
export const defaultLocale: Locale = "en";
export const LOCALE_COOKIE = "zeno_locale";

export function isLocale(value: unknown): value is Locale {
  return typeof value === "string" && (locales as readonly string[]).includes(value);
}

/**
 * Which of `available` to show: the one the person chose (the cookie), otherwise the best match in
 * what their browser prefers (`Accept-Language`, with its q-values), otherwise `fallback`.
 * "hi-IN" and "hi" are the same language; one we do not have is skipped, not an error.
 * Pure, and tested with more languages than the app has, so it is right the day it has them.
 */
export function negotiate(
  cookie: string | undefined,
  acceptLanguage: string | null | undefined,
  available: readonly string[],
  fallback: string,
): string {
  if (cookie !== undefined && available.includes(cookie)) return cookie;
  const preferences = (acceptLanguage ?? "")
    .split(",")
    .map((part) => {
      const [tag, ...params] = part.trim().split(";");
      const q = params.map((p) => p.trim()).find((p) => p.startsWith("q="));
      const weight = q === undefined ? 1 : Number(q.slice(2));
      return { language: tag.trim().toLowerCase().split("-")[0], weight };
    })
    .filter((p) => p.language !== "" && Number.isFinite(p.weight) && p.weight > 0)
    // Stable: equal weights keep the order the browser gave them.
    .sort((a, b) => b.weight - a.weight);
  for (const { language } of preferences) {
    if (available.includes(language)) return language;
  }
  return fallback;
}

export function pickLocale(cookie: string | undefined, acceptLanguage: string | null | undefined): Locale {
  return negotiate(cookie, acceptLanguage, locales, defaultLocale) as Locale;
}

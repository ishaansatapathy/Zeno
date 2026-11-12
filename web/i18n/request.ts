import { cookies, headers } from "next/headers";
import { getRequestConfig } from "next-intl/server";

import { LOCALE_COOKIE, pickLocale } from "@/lib/i18n/locale";
import { TIME_ZONE } from "@/lib/i18n/format";

/**
 * Per-request language, with no locale in the URL: the person's choice (a cookie), else their
 * browser's preference. Server components render the right language on the first byte, so there
 * is no flash of English and no mismatch for the client to repair.
 */
export default getRequestConfig(async () => {
  const locale = pickLocale(cookies().get(LOCALE_COOKIE)?.value, headers().get("accept-language"));
  return {
    locale,
    timeZone: TIME_ZONE,
    messages: (await import(`../messages/${locale}.json`)).default,
  };
});

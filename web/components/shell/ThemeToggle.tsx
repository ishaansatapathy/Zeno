"use client";

import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";

import { THEME_COOKIE } from "@/lib/theme/cookie";

type Choice = "system" | "light" | "dark";

function apply(choice: Choice) {
  const root = document.documentElement;
  if (choice === "system") {
    delete root.dataset.theme;
    document.cookie = `${THEME_COOKIE}=; path=/; max-age=0; samesite=lax`;
  } else {
    root.dataset.theme = choice;
    document.cookie = `${THEME_COOKIE}=${choice}; path=/; max-age=31536000; samesite=lax`;
  }
}

/** Light, dark, or whatever the device says. The choice is a cookie, so the server renders it. */
export function ThemeToggle({ initial }: { initial: Choice }) {
  const t = useTranslations("common");
  const [choice, setChoice] = useState<Choice>(initial);
  useEffect(() => apply(choice), [choice]);

  return (
    <label className="flex items-center gap-2 text-sm">
      <span className="sr-only">{t("theme")}</span>
      <select
        value={choice}
        onChange={(event) => setChoice(event.target.value as Choice)}
        className="min-h-9 rounded-control border border-border-strong bg-bg px-2 text-text"
      >
        <option value="system">{t("themeSystem")}</option>
        <option value="light">{t("themeLight")}</option>
        <option value="dark">{t("themeDark")}</option>
      </select>
    </label>
  );
}

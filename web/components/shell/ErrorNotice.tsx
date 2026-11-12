"use client";

import { useTranslations } from "next-intl";

import { ApiError } from "@/lib/api/errors";
import { errorMessage } from "@/lib/api/messages";

/**
 * A failed call, explained in the person's language, with the request's reference quoted so that
 * support can find it in the logs. Announced to screen readers as it appears.
 */
export function ErrorNotice({ error }: { error: unknown }) {
  const t = useTranslations("errors");
  const { key, values } = errorMessage(error);
  const requestId = error instanceof ApiError ? error.requestId : undefined;
  return (
    <div role="alert" className="rounded-control border border-danger bg-surface p-3 text-sm">
      <p className="font-medium text-danger">{t(key, values)}</p>
      {requestId && <p className="mt-1 text-text-muted">{t("reference", { id: requestId })}</p>}
    </div>
  );
}

import { getTranslations } from "next-intl/server";

import { AccountView } from "@/components/auth/AccountView";

export async function generateMetadata() {
  const t = await getTranslations("account");
  return { title: t("title") };
}

export default async function AccountPage() {
  const t = await getTranslations("account");
  return (
    <div className="mx-auto flex max-w-xl flex-col gap-6">
      <h1 className="text-3xl font-bold tracking-tight">{t("title")}</h1>
      <AccountView />
    </div>
  );
}

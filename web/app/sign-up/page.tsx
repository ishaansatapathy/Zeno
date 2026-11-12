import { getTranslations } from "next-intl/server";

import { SignUpForm } from "@/components/auth/SignUpForm";
import { Card } from "@/components/ui/Card";

export async function generateMetadata() {
  const t = await getTranslations("auth.signUp");
  return { title: t("title") };
}

export default async function SignUpPage() {
  const t = await getTranslations("auth.signUp");
  return (
    <div className="mx-auto flex max-w-md flex-col gap-6">
      <h1 className="text-3xl font-bold tracking-tight">{t("title")}</h1>
      <Card>
        <SignUpForm />
      </Card>
    </div>
  );
}

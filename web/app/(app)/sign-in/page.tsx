import { getTranslations } from "next-intl/server";

import { SignInForm } from "@/components/auth/SignInForm";
import { Card } from "@/components/ui/Card";

export async function generateMetadata() {
  const t = await getTranslations("auth.signIn");
  return { title: t("title") };
}

export default async function SignInPage() {
  const t = await getTranslations("auth.signIn");
  return (
    <div className="mx-auto flex max-w-md flex-col gap-6">
      <h1 className="display text-5xl">{t("title")}</h1>
      <Card>
        <SignInForm />
      </Card>
    </div>
  );
}

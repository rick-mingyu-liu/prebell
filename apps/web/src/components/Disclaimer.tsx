import { getTranslations } from "next-intl/server";

export default async function Disclaimer() {
  const t = await getTranslations();
  return <p className="disclaimer">{t("disclaimer")}</p>;
}

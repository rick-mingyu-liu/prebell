import { getTranslations } from "next-intl/server";
import type { PostChart } from "@/lib/earnings";

/** A post's GAAP charts, served from R2 by /charts/... (never foreign URLs). */
export default async function Charts({ charts }: { charts: PostChart[] }) {
  if (charts.length === 0) return null;
  const t = await getTranslations("earnings");
  return (
    <section className="charts" aria-labelledby="charts">
      <h2 id="charts">{t("charts")}</h2>
      {charts.map((c) => (
        <figure key={c.name}>
          {/* A static same-site SVG; next/image would add nothing here. */}
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={c.src} alt={c.alt} width={640} height={280} loading="lazy" />
          <figcaption>{c.alt}</figcaption>
        </figure>
      ))}
      <p className="muted">{t("chartsNote")}</p>
    </section>
  );
}

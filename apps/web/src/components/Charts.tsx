import type { Locale } from "../i18n/routing";
import type { PostChart } from "../lib/earnings";
import { translator } from "../lib/translator";

/** A post's GAAP charts, served from R2 by /charts/... (never foreign URLs). */
export default function Charts({ charts, locale }: { charts: PostChart[]; locale: Locale }) {
  if (charts.length === 0) return null;
  const t = translator(locale);
  return (
    <section className="charts" aria-labelledby="charts">
      <h2 id="charts">{t("earnings.charts")}</h2>
      {charts.map((c) => (
        <figure key={c.name}>
          {/* A static same-site SVG; next/image would add nothing here. */}
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={c.src} alt={c.alt} width={640} height={280} loading="lazy" />
          <figcaption>{c.alt}</figcaption>
        </figure>
      ))}
      <p className="muted">{t("earnings.chartsNote")}</p>
    </section>
  );
}

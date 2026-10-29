import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { getTranslations, setRequestLocale } from "next-intl/server";
import LangSwitch from "@/components/LangSwitch";
import PostList from "@/components/PostList";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { listTickerCalendar, listTickerPosts, normalizeTicker, tickerKnown } from "@/lib/earnings";
import { pairedAlternates } from "@/lib/seo";
import { formatDate } from "@/lib/time";

export async function generateMetadata({ params }: PageProps<"/[locale]/t/[ticker]">): Promise<Metadata> {
  const { locale, ticker: raw } = await params;
  const ticker = normalizeTicker(raw);
  if (!ticker) return {};
  const t = await getTranslations({ locale, namespace: "earnings" });
  return {
    title: t("tickerTitle", { ticker }),
    alternates: pairedAlternates(locale, `/t/${encodeURIComponent(ticker)}`),
  };
}

export default async function TickerArchive({ params }: PageProps<"/[locale]/t/[ticker]">) {
  const { ticker: raw } = await params;
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const ticker = normalizeTicker(raw);
  const db = getDb();
  if (!ticker || !(await tickerKnown(db, ticker))) notFound();
  const t = await getTranslations("earnings");
  const [posts, dates] = await Promise.all([listTickerPosts(db, ticker, locale), listTickerCalendar(db, ticker)]);
  const status: Record<string, string> = {
    unconfirmed: t("unconfirmed"),
    confirmed: t("confirmed"),
    reported: t("reported"),
  };
  const slot: Record<string, string> = { bmo: t("bmo"), amc: t("amc"), unknown: t("timeUnknown") };

  return (
    <>
      <div className="page-tools">
        <h1>{t("tickerTitle", { ticker })}</h1>
        <LangSwitch locale={locale} path={`/t/${encodeURIComponent(ticker)}`} />
      </div>
      <section aria-labelledby="checks">
        <h2 id="checks">{t("checks")}</h2>
        {posts.length === 0 ? <p className="muted">{t("noChecks")}</p> : <PostList locale={locale} items={posts} />}
      </section>
      {dates.length > 0 && (
        <section aria-labelledby="dates">
          <h2 id="dates">{t("dates")}</h2>
          <ul className="post-list">
            {dates.map((d) => (
              <li key={d.date}>
                <time dateTime={d.date}>{formatDate(d.date, locale)}</time> {slot[d.slot]} ·{" "}
                <span className={`badge badge-${d.status}`}>{status[d.status]}</span>
                {d.postSlug && (
                  <>
                    {" · "}
                    <Link href={`/${locale}/p/${d.postSlug}`}>{t("readCheck")}</Link>
                  </>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}
      <p>
        <Link href={`/${locale}/earnings`}>{t("calendarTitle")} →</Link>
      </p>
    </>
  );
}

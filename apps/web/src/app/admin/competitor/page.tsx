import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getCloudflareContext } from "@opennextjs/cloudflare";
import { getDb } from "@/lib/db";
import { getLatestReport, listRecentItems, tokenMatches, type ItemRef } from "@/lib/competitor";
import { formatEt } from "@/lib/time";

export const metadata: Metadata = { title: "Competitor watch" };

// Their links: no referrer, so the token in this page's URL never leaves.
function Ext({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <a href={href} target="_blank" rel="noreferrer noopener">
      {children}
    </a>
  );
}

function Item({ i }: { i: ItemRef }) {
  return (
    <>
      <Ext href={i.link}>{i.title}</Ext>
      <span className="muted">
        {i.category && ` · ${i.category}`} · {formatEt(i.published_at, "en")}
      </span>
    </>
  );
}

/** Internal only (spec §3.6, §7): 404 unless `?token=` matches the ADMIN_TOKEN secret. */
export default async function CompetitorPage({ searchParams }: PageProps<"/admin/competitor">) {
  const { token } = await searchParams;
  if (!tokenMatches(token, getCloudflareContext().env.ADMIN_TOKEN)) notFound();

  const db = getDb();
  const [latest, items] = await Promise.all([getLatestReport(db), listRecentItems(db, 100)]);
  const r = latest?.report;

  return (
    <>
      <h1>Competitor watch</h1>
      <p className="muted">
        Internal only. AlphaLab feed metadata (titles, links, times, categories, title tickers). Never an input to our
        articles.
      </p>

      {!latest || !r ? (
        <p>No report yet.</p>
      ) : (
        <>
          <h2>Report {latest.reportDate}</h2>
          <p className="muted">
            Window {formatEt(r.window_start, "en")} to {formatEt(r.window_end, "en")}: {r.their_item_count} of their
            items, {r.our_post_count} of our posts{r.our_slugs.length > 0 && ` (${r.our_slugs.join(", ")})`}.
          </p>

          <h3>Not covered by us ({r.uncovered.length})</h3>
          {r.uncovered.length === 0 ? (
            <p>None.</p>
          ) : (
            <ul>
              {r.uncovered.map((g) => (
                <li key={g.ticker}>
                  <strong>{g.ticker}</strong>
                  <ul>
                    {g.items.map((i) => (
                      <li key={i.link}>
                        <Item i={i} />
                      </li>
                    ))}
                  </ul>
                </li>
              ))}
            </ul>
          )}

          <h3>Topics without tickers ({r.topics.length})</h3>
          {r.topics.length === 0 ? (
            <p>None.</p>
          ) : (
            <ul>
              {r.topics.map((i) => (
                <li key={i.link}>
                  <Item i={i} />
                </li>
              ))}
            </ul>
          )}

          <h3>Covered by both ({r.covered.length})</h3>
          <p>{r.covered.length === 0 ? "None." : r.covered.join(", ")}</p>
        </>
      )}

      <h2>Recent items ({items.length})</h2>
      {items.length === 0 ? (
        <p>None stored.</p>
      ) : (
        <ul>
          {items.map((i) => (
            <li key={i.link}>
              <Ext href={i.link}>{i.title}</Ext>
              <span className="muted">
                {i.tickers.length > 0 && ` · ${i.tickers.join(" ")}`}
                {i.category && ` · ${i.category}`}
                {i.publishedAt ? ` · ${formatEt(i.publishedAt, "en")}` : " · no date"}
              </span>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

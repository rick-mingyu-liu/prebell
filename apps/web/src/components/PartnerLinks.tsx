"use client";

import { useEffect, useState } from "react";
import type { PartnerGroup, PartnersResponse } from "../lib/partners";

const GROUPS: PartnerGroup[] = ["brokers", "crypto"];

export type PartnerLabels = Record<PartnerGroup | "disclosure", string>;

const isResponse = (v: unknown): v is PartnersResponse =>
  typeof v === "object" && v !== null && GROUPS.every((g) => Array.isArray((v as Record<string, unknown>)[g]));

let pending: Promise<PartnersResponse | null> | undefined;

/**
 * The visitor's links, fetched once per page load and shared by every mount. Resolves null on any
 * failure (and forgets it, so a later mount retries).
 */
export function loadPartners(fetchImpl: (url: string) => Promise<Response> = fetch) {
  pending ??= fetchImpl("/api/partners")
    .then((res) => (res.ok ? res.json() : null))
    .then((body: unknown) => (isResponse(body) ? body : null))
    .catch(() => null)
    .then((data) => {
      if (!data) pending = undefined;
      return data;
    });
  return pending;
}

/** One box per group that has links, then the disclosure; nothing at all when there are no links. */
export function PartnerBoxes({ data, labels }: { data: PartnersResponse | null; labels: PartnerLabels }) {
  const groups = data ? GROUPS.filter((g) => data[g].length > 0) : [];
  if (!data || groups.length === 0) return null;
  return (
    <>
      {groups.map((g) => (
        <section key={g} className="side-box">
          <h2>{labels[g]}</h2>
          <ul>
            {data[g].map((p) => (
              <li key={p.id}>
                <a href={p.url} target="_blank" rel="sponsored noopener noreferrer">
                  {p.name}
                  <span aria-hidden="true">↗</span>
                </a>
              </li>
            ))}
          </ul>
        </section>
      ))}
      <p className="side-disc">{labels.disclosure}</p>
    </>
  );
}

/**
 * Country-gated invite links. Pages are edge-cached and country-neutral, so the links are fetched
 * per visitor after load. Nothing renders on the server, while loading or on error (fails closed).
 */
export default function PartnerLinks({ labels }: { labels: PartnerLabels }) {
  const [data, setData] = useState<PartnersResponse | null>(null);

  useEffect(() => {
    let live = true;
    loadPartners().then((d) => {
      if (live) setData(d);
    });
    return () => {
      live = false;
    };
  }, []);

  return <PartnerBoxes data={data} labels={labels} />;
}

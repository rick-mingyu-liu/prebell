"use client";

import { useEffect, useState } from "react";
import type { PartnerGroup, PartnersResponse } from "../lib/partners";

const GROUPS: PartnerGroup[] = ["brokers", "crypto"];

const isResponse = (v: unknown): v is PartnersResponse =>
  typeof v === "object" && v !== null && GROUPS.every((g) => Array.isArray((v as Record<string, unknown>)[g]));

/** One box per group that has links; nothing at all when every group is empty. */
export function PartnerBoxes({
  data,
  titles,
}: {
  data: PartnersResponse;
  titles: Record<PartnerGroup, string>;
}) {
  return GROUPS.filter((g) => data[g].length > 0).map((g) => (
    <section key={g} className="side-box">
      <h2>{titles[g]}</h2>
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
  ));
}

/**
 * Country-gated invite links. Pages are edge-cached and country-neutral, so the links are fetched
 * per visitor after load. Nothing renders on the server, while loading or on error (fails closed).
 */
export default function PartnerLinks({ titles }: { titles: Record<PartnerGroup, string> }) {
  const [data, setData] = useState<PartnersResponse | null>(null);

  useEffect(() => {
    let live = true;
    fetch("/api/partners")
      .then((res) => (res.ok ? res.json() : null))
      .then((body: unknown) => {
        if (live && isResponse(body)) setData(body);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  return data && <PartnerBoxes data={data} titles={titles} />;
}

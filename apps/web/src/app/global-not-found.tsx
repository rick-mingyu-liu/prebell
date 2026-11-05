import type { Metadata } from "next";
import Link from "next/link";
import { inter, newsreader } from "@/lib/fonts";
import en from "../../messages/en.json";
import zh from "../../messages/zh.json";
import "./globals.css";

// Served server-side for unmatched routes only (e.g. /en/xyz). notFound() thrown by the [locale]
// layout or a page still goes through [locale]/not-found.tsx, which Next 16 renders client-side
// because the root layout is the dynamic [locale] segment. Bilingual since it receives no params.
export const metadata: Metadata = { title: `${en.notFound.title} · ${zh.notFound.title}` };

export default function GlobalNotFound() {
  return (
    <html lang="en" className={`${inter.variable} ${newsreader.variable}`}>
      <body>
        <div className="wrap">
          <header className="masthead">
            <Link href="/en" className="masthead-name">
              PreBell <small lang="zh-Hans">{zh.site.wordmarkSub}</small>
            </Link>
          </header>
        </div>
        <main className="wrap">
          <section className="notfound">
            <p className="kicker" aria-hidden="true">
              404
            </p>
            <h1>{en.notFound.title}</h1>
            <p>
              <Link className="more" href="/en">
                {en.notFound.back} →
              </Link>
            </p>
            <div lang="zh-Hans">
              <h1>{zh.notFound.title}</h1>
              <p>
                <Link className="more" href="/zh">
                  {zh.notFound.back} →
                </Link>
              </p>
            </div>
          </section>
        </main>
      </body>
    </html>
  );
}

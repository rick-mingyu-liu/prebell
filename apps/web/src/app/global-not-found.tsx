import type { Metadata } from "next";
import Link from "next/link";
import { inter } from "@/lib/fonts";
import en from "../../messages/en.json";
import zh from "../../messages/zh.json";
import "./globals.css";

// Served server-side for unmatched routes only (e.g. /en/xyz). notFound() thrown by the [locale]
// layout or a page still goes through [locale]/not-found.tsx, which Next 16 renders client-side
// because the root layout is the dynamic [locale] segment. Bilingual since it receives no params.
export const metadata: Metadata = { title: `${en.notFound.title} · ${zh.notFound.title}` };

export default function GlobalNotFound() {
  return (
    <html lang="en" className={inter.variable}>
      <body>
        <main>
          <h1>{en.notFound.title}</h1>
          <p>
            <Link href="/en">{en.notFound.back}</Link>
          </p>
          <div lang="zh-Hans">
            <h1>{zh.notFound.title}</h1>
            <p>
              <Link href="/zh">{zh.notFound.back}</Link>
            </p>
          </div>
        </main>
        <footer className="site-footer">
          <p className="disclaimer">{en.disclaimer}</p>
          <p className="disclaimer" lang="zh-Hans">{zh.disclaimer}</p>
        </footer>
      </body>
    </html>
  );
}

import type { Metadata } from "next";
import Link from "next/link";
import { inter } from "@/lib/fonts";
import en from "../../messages/en.json";
import zh from "../../messages/zh.json";
import "./globals.css";

// The root layout is the dynamic [locale] segment, so a per-locale not-found.tsx cannot render
// server-side (Next falls back to an empty error shell). This page serves every 404 instead,
// in both languages since it receives no params.
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

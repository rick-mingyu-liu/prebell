import type { Metadata } from "next";
import { inter } from "@/lib/fonts";
import "../globals.css";

// Internal pages: a separate root layout, outside the public [locale] tree, never indexed.
export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "PreBell admin",
  robots: { index: false, follow: false, nocache: true },
  referrer: "no-referrer",
};

export default function AdminLayout({ children }: LayoutProps<"/admin">) {
  return (
    <html lang="en" className={inter.variable}>
      <body>
        <main>{children}</main>
      </body>
    </html>
  );
}

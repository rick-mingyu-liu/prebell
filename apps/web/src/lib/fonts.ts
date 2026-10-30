import localFont from "next/font/local";

// Inter (variable, Latin subset) from @fontsource-variable/inter, self-hosted by next/font.
// Chinese text falls back to the system CJK stack in globals.css.
export const inter = localFont({
  src: "../../node_modules/@fontsource-variable/inter/files/inter-latin-wght-normal.woff2",
  weight: "100 900",
  style: "normal",
  display: "swap",
  variable: "--font-inter",
});

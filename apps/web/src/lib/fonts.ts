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

// Newsreader (variable, Latin subset) from @fontsource-variable/newsreader for the masthead, headlines
// and section titles. Chinese pages use the system serif stack from globals.css instead.
export const newsreader = localFont({
  src: "../../node_modules/@fontsource-variable/newsreader/files/newsreader-latin-wght-normal.woff2",
  weight: "200 800",
  style: "normal",
  display: "swap",
  variable: "--font-serif",
});

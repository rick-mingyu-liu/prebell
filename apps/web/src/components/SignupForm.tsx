import "server-only";
import { signup as en } from "../../messages/en.json";
import { signup as zh } from "../../messages/zh.json";
import type { Locale } from "../i18n/routing";
import SignupFormClient from "./SignupFormClient";

/**
 * Weekly-digest signup, rendered on the server: it picks the page's language and hands the client
 * form only that language's signup strings. The digest is one English email to every subscriber;
 * pages in both languages show the form, and the Chinese heading and copy say the email is in English.
 */
export default function SignupForm({ source, locale }: { source: "post" | "home" | "about"; locale: Locale }) {
  return <SignupFormClient source={source} m={locale === "zh" ? zh : en} />;
}

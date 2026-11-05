"use client";

import { type FormEvent, useState } from "react";
import { signup as signupEn } from "../../messages/en.json";
import { signup as signupZh } from "../../messages/zh.json";
import type { Locale } from "../i18n/routing";

type Status = "idle" | "loading" | "done" | "error" | "invalid";

/**
 * Weekly-digest signup: email field, Subscribe button and an inline result. Posts JSON to
 * /api/subscribe; the honeypot `website` field stays out of sight (not display:none, which bots skip).
 * Only the signup strings are pulled in, not the whole message catalogue.
 */
export default function SignupForm({ source, locale }: { source: "post" | "home" | "about"; locale: Locale }) {
  const m = locale === "zh" ? signupZh : signupEn;
  const [status, setStatus] = useState<Status>("idle");

  async function onSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    const email = String(data.get("email") ?? "").trim();
    if (!email) return setStatus("invalid");
    setStatus("loading");
    try {
      const res = await fetch("/api/subscribe", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email, source, lang: locale, website: String(data.get("website") ?? "") }),
      });
      setStatus(res.ok ? "done" : res.status === 400 ? "invalid" : "error");
    } catch {
      setStatus("error");
    }
  }

  const message = status === "done" ? m.success : status === "error" ? m.error : status === "invalid" ? m.invalid : null;

  return (
    <section className="signup">
      <h2 className="signup-title">{m.heading}</h2>
      <p className="signup-copy">{m.copy}</p>
      {status !== "done" && (
        <form className="signup-form" onSubmit={onSubmit} noValidate>
          <label className="sr-only" htmlFor={`signup-email-${source}`}>
            {m.emailLabel}
          </label>
          <input
            id={`signup-email-${source}`}
            type="email"
            name="email"
            autoComplete="email"
            placeholder={m.placeholder}
            required
          />
          <input className="sr-only" type="text" name="website" tabIndex={-1} autoComplete="off" aria-hidden="true" />
          <button type="submit" disabled={status === "loading"}>
            {status === "loading" ? m.sending : m.submit}
          </button>
        </form>
      )}
      {message && (
        <p className={status === "done" ? "signup-msg" : "signup-msg err"} role={status === "done" ? "status" : "alert"}>
          {message}
        </p>
      )}
      <p className="signup-manage">
        <a href="https://buttondown.com/portal" target="_blank" rel="noopener noreferrer">
          {m.manage}
        </a>
      </p>
    </section>
  );
}

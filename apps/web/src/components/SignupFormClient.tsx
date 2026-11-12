"use client";

import { type FormEvent, useState } from "react";

type Status = "idle" | "loading" | "done" | "already" | "error" | "invalid";
/** The signup strings of one language. A type-only import: no message catalogue reaches the client bundle. */
export type SignupMessages = (typeof import("../../messages/en.json"))["signup"];

/** The form's state after /api/subscribe answers: a 200 is done (or already, when the route says so). */
export async function statusOf(res: Response): Promise<Status> {
  if (!res.ok) return res.status === 400 ? "invalid" : "error";
  const body: unknown = await res.json().catch(() => null);
  return typeof body === "object" && body !== null && (body as { already?: unknown }).already === true
    ? "already"
    : "done";
}

/** The inline line under the form for `status`, or null while idle or sending. */
export function messageFor(status: Status, m: SignupMessages): string | null {
  switch (status) {
    case "done":
      return m.success;
    case "already":
      return m.already;
    case "error":
      return m.error;
    case "invalid":
      return m.invalid;
    default:
      return null;
  }
}

/**
 * Weekly-digest signup: email field, Subscribe button and an inline result. Posts JSON to
 * /api/subscribe; the honeypot `website` field stays out of sight (not display:none, which bots skip).
 * `source` names the page (the route checks it) and keeps the field ids unique. The strings (`m`)
 * are chosen on the server by `SignupForm` and passed as props, so no message catalogue reaches the
 * client bundle (a named import from a JSON file would ship the whole file).
 */
export default function SignupFormClient({ source, m }: { source: "post" | "home" | "about"; m: SignupMessages }) {
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
        body: JSON.stringify({ email, source, website: String(data.get("website") ?? "") }),
      });
      setStatus(await statusOf(res));
    } catch {
      setStatus("error");
    }
  }

  const message = messageFor(status, m);
  const settled = status === "done" || status === "already";

  return (
    <section className="signup">
      <h2 className="signup-title">{m.heading}</h2>
      <p className="signup-copy">{m.copy}</p>
      {!settled && (
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
        <p className={settled ? "signup-msg" : "signup-msg err"} role={settled ? "status" : "alert"}>
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

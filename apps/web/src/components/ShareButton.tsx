"use client";

import { useState, useSyncExternalStore } from "react";

const noop = () => () => {};

/**
 * Share this page: the system share sheet where there is one, else the link copied to the clipboard,
 * else the link written out to copy by hand. The button is rendered hidden and shown once the script
 * runs, so a page without script has no dead control.
 */
export default function ShareButton({
  path,
  title,
  label,
  copied,
  failed,
}: {
  /** The page's path and query, made absolute against the current origin. */
  path: string;
  title: string;
  label: string;
  copied: string;
  failed: string;
}) {
  const ready = useSyncExternalStore(
    noop,
    () => true,
    () => false,
  );
  const [status, setStatus] = useState<{ ok: boolean; url: string } | null>(null);

  async function share() {
    // Clear the last result first, so a repeat is announced again and an old "copied" never lingers.
    setStatus(null);
    const url = new URL(path, location.origin).href;
    if (typeof navigator.share === "function") {
      try {
        await navigator.share({ title, url });
        return;
      } catch (err) {
        // The person closed the sheet: nothing to do. Any other failure falls through to copying.
        if (err instanceof DOMException && err.name === "AbortError") return;
      }
    }
    try {
      await navigator.clipboard.writeText(url);
      setStatus({ ok: true, url });
    } catch {
      setStatus({ ok: false, url });
    }
  }

  return (
    <span className="cal-share">
      <button type="button" className="cal-btn cal-share-btn" hidden={!ready} onClick={share}>
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
          <path
            d="M8 10V2M5 4.5 8 1.5l3 3M3.5 7.5v6h9v-6"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
        {label}
      </button>
      <span className="cal-share-msg" role="status">
        {status?.ok && copied}
        {status && !status.ok && (
          <>
            {failed} <span className="cal-share-url">{status.url}</span>
          </>
        )}
      </span>
    </span>
  );
}

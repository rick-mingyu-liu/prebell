"use client";

import { useEffect } from "react";

/**
 * On a phone the calendar board is a sideways scroll-snap row that starts at Monday; this moves it to
 * today's column once on load. Browsers with `scroll-initial-target` already start there, and without
 * script the row simply starts at Monday. A link to a day (#d-…) wins over today.
 */
export default function BoardStart({ target }: { target: string }) {
  useEffect(() => {
    const col = document.getElementById(target);
    const board = col?.parentElement;
    if (!col || !board || board.scrollWidth <= board.clientWidth || location.hash) return;
    const pad = parseFloat(getComputedStyle(board).scrollPaddingLeft) || 0;
    board.scrollTo({ left: col.offsetLeft - pad, behavior: "instant" });
  }, [target]);
  return null;
}

"use client";

import { useEffect, useRef } from "react";

/**
 * On a phone the calendar board is a sideways scroll-snap row that starts at Monday; this moves it to
 * today's column once on load. Browsers with `scroll-initial-target` already start there, and without
 * script the row simply starts at Monday. It leaves the row alone when it is not a scroller (a desktop),
 * when a link to a day (#d-…) chose the column, and when the reader has already moved it. The jump is
 * instant, never animated.
 */
export default function BoardStart({ target }: { target: string }) {
  const done = useRef(false);
  useEffect(() => {
    if (done.current) return;
    done.current = true;
    const col = document.getElementById(target);
    const board = col?.parentElement;
    if (!col || !board || board.scrollWidth <= board.clientWidth || location.hash || board.scrollLeft > 0) return;
    const pad = parseFloat(getComputedStyle(board).scrollPaddingLeft) || 0;
    board.scrollTo({ left: col.offsetLeft - pad, behavior: "instant" });
  }, [target]);
  return null;
}

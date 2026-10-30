/**
 * A GFM table's delimiter row, e.g. `|---|---:|`, matched against the trimmed line (no trailing `\s*`,
 * so it runs in linear time). It needs a pipe, so `---` alone stays a rule or setext heading.
 */
const DELIMITER = /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/;
const isDelimiter = (line: string | undefined) =>
  line !== undefined && line.includes("|") && DELIMITER.test(line.trim());
const isTableStart = (lines: string[], i: number) => lines[i].includes("|") && isDelimiter(lines[i + 1]);
const FENCE = /^ {0,3}(```|~~~)/;
/** A line that opens a block other than a paragraph or a table row. */
const BLOCK = /^ {0,3}(#|>|[-*+]\s|\d+[.)]\s|```|~~~)/;
const ATX_TITLE = /^ {0,3}#\s/;
const SETEXT_TITLE = /^ {0,3}=+\s*$/;
const words = (s: string) => s.split(/\s+/).filter(Boolean).join(" ");

/**
 * A brief's markdown without what the page shows above it: the leading title (`# ` or setext), the
 * summary paragraph right after it, and every GFM table outside code fences. The first paragraph is
 * dropped only when it reads the same as `summary` (when one is given). The `## ` sections, the
 * sources and the closing disclaimer stay.
 */
export function trimStory(md: string, summary?: string): string {
  const lines = md.replace(/^﻿/, "").replace(/\r\n?/g, "\n").split("\n");
  let i = 0;
  const skipBlank = () => {
    while (i < lines.length && !lines[i].trim()) i++;
  };
  skipBlank();
  if (i < lines.length && ATX_TITLE.test(lines[i])) i += 1;
  else if (i < lines.length && lines[i].trim() && SETEXT_TITLE.test(lines[i + 1] ?? "")) i += 2;
  skipBlank();

  // The summary paragraph runs to a blank line or the start of another block.
  let end = i;
  while (end < lines.length && lines[end].trim() && !BLOCK.test(lines[end]) && !isTableStart(lines, end)) end++;
  if (end > i && (summary === undefined || words(lines.slice(i, end).join(" ")) === words(summary))) i = end;

  const out: string[] = [];
  let inFence = false;
  while (i < lines.length) {
    const line = lines[i];
    if (FENCE.test(line)) inFence = !inFence;
    else if (!inFence && isTableStart(lines, i)) {
      // A table runs to the next blank line or the start of another block.
      i += 2;
      while (i < lines.length && lines[i].trim() && !BLOCK.test(lines[i])) i++;
      continue;
    }
    out.push(line);
    i++;
  }
  const text = out.join("\n").replace(/\n{3,}/g, "\n\n").trim();
  return text ? `${text}\n` : "";
}

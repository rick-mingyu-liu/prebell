/** A GFM table's delimiter row, e.g. `|---|---:|`; it needs a pipe, so `---` alone stays a rule or setext heading. */
const DELIMITER = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;
const isDelimiter = (line: string | undefined) => line !== undefined && line.includes("|") && DELIMITER.test(line);
const isTableStart = (lines: string[], i: number) => lines[i].includes("|") && isDelimiter(lines[i + 1]);
/** A line that opens something other than a paragraph. */
const BLOCK_START = /^\s*(#|\||>|[-*+]\s|\d+[.)]\s|```|~~~)/;

/**
 * A brief's markdown without what the page shows above it: the leading `# ` title, the summary
 * (the paragraph right after the title) and every GFM table. The `## ` sections, the sources and
 * the closing disclaimer stay.
 */
export function trimStory(md: string): string {
  const lines = md.replace(/\r\n?/g, "\n").split("\n");
  let i = 0;
  const skipBlank = () => {
    while (i < lines.length && !lines[i].trim()) i++;
  };
  skipBlank();
  if (/^#\s/.test(lines[i] ?? "")) {
    i++;
    skipBlank();
  }
  if (i < lines.length && !BLOCK_START.test(lines[i]) && !isTableStart(lines, i)) {
    while (i < lines.length && lines[i].trim()) i++;
  }
  const out: string[] = [];
  while (i < lines.length) {
    if (isTableStart(lines, i)) {
      // A table runs to the next blank line or heading.
      i += 2;
      while (i < lines.length && lines[i].trim() && !/^\s*#/.test(lines[i])) i++;
      continue;
    }
    out.push(lines[i++]);
  }
  const text = out.join("\n").replace(/\n{3,}/g, "\n\n").trim();
  return text ? `${text}\n` : "";
}

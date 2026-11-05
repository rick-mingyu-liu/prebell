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

/** A post body as lines: no BOM, LF line ends. */
const linesOf = (md: string) => md.replace(/^\uFEFF/, "").replace(/\r\n?/g, "\n").split("\n");
const joined = (lines: string[]) => {
  const text = lines.join("\n").replace(/\n{3,}/g, "\n\n").trim();
  return text ? `${text}\n` : "";
};

/** The index of the first line after the leading title (`# ` or setext) and, when it matches, the summary. */
function headEnd(lines: string[], summary?: string): number {
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
  return i;
}

/**
 * A post's markdown without the leading title (`# ` or setext) and the summary paragraph right after
 * it, which the page shows as its headline and lede. The first paragraph is dropped only when it reads
 * the same as `summary` (when one is given). Tables and everything else stay.
 */
export function trimHead(md: string, summary?: string): string {
  const lines = linesOf(md);
  return joined(lines.slice(headEnd(lines, summary)));
}

/**
 * A brief's markdown without what the page shows above it: the leading title (`# ` or setext), the
 * summary paragraph right after it, and every GFM table outside code fences. The first paragraph is
 * dropped only when it reads the same as `summary` (when one is given). The `## ` sections, the
 * sources and the closing disclaimer stay.
 */
export function trimStory(md: string, summary?: string): string {
  const lines = linesOf(md);
  let i = headEnd(lines, summary);
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
  return joined(out);
}

/** Fence state per line, so headings and paragraphs inside code never count. */
function fencedLines(lines: string[]): boolean[] {
  const fenced: boolean[] = [];
  let inFence = false;
  for (const line of lines) {
    const opens = FENCE.test(line);
    fenced.push(inFence || opens);
    if (opens) inFence = !inFence;
  }
  return fenced;
}

/** The generators' sources heading, EN or ZH. */
const SOURCES = /^ {0,3}##\s+(Sources|来源)\s*#*\s*$/;
const SECTION = /^ {0,3}##\s/;
/** A line that can open a plain paragraph: not indented as code or a list continuation. */
const PARAGRAPH_START = /^ {0,3}\S/;

export interface StoryTail {
  /** The markdown before the sources section and the disclaimer. */
  body: string;
  /** The sources section's content (its list), without the heading; "" when there is none. */
  sources: string;
  /** The closing paragraph, as written; "" when there is none. */
  disclaimer: string;
}

/**
 * A post body split into what the page shows apart: the `## Sources` / `## 来源` section and the closing
 * disclaimer, which every generator writes last. The disclaimer is the final plain paragraph after at
 * least one `## ` section; a final list, table, heading or fenced block is never taken for it.
 */
export function splitTail(md: string): StoryTail {
  const lines = linesOf(md);
  const fenced = fencedLines(lines);

  let firstSection = -1;
  let sourcesAt = -1;
  lines.forEach((line, i) => {
    if (fenced[i]) return;
    if (firstSection < 0 && SECTION.test(line)) firstSection = i;
    if (SOURCES.test(line)) sourcesAt = i;
  });

  // The last block: back from the end to the blank line before it.
  let end = lines.length;
  while (end > 0 && !lines[end - 1].trim()) end--;
  let start = end;
  while (start > 0 && lines[start - 1].trim()) start--;
  const block = lines.slice(start, end);
  const isDisclaimer =
    block.length > 0 &&
    firstSection >= 0 &&
    firstSection < start &&
    !fenced.slice(start, end).some(Boolean) &&
    PARAGRAPH_START.test(block[0]) &&
    !BLOCK.test(block[0]) &&
    !block.some((l) => l.includes("|") && isDelimiter(l));
  const tailAt = isDisclaimer ? start : lines.length;

  const hasSources = sourcesAt >= 0 && sourcesAt < tailAt;
  return {
    body: joined(lines.slice(0, hasSources ? sourcesAt : tailAt)),
    sources: hasSources ? joined(lines.slice(sourcesAt + 1, tailAt)) : "",
    disclaimer: isDisclaimer ? block.map((l) => l.trim()).join(" ") : "",
  };
}

/** The generators' movers headings: premarket and recap, EN and ZH. */
const MOVERS = /^ {0,3}##\s+(Biggest premarket movers|Notable movers|盘前异动|个股异动)\s*#*\s*$/;

export interface MoversSplit {
  /** The markdown above the movers heading; the whole input when there is none. */
  before: string;
  /** The movers heading's text, as written; null when there is none. */
  heading: string | null;
  /** The markdown after the movers heading line (its prose and any later sections); "" when there is none. */
  after: string;
}

/**
 * A story body split at its movers section's `## ` heading (EN or ZH, premarket or recap), so the page
 * can put the movers table under that heading, right before its prose. Headings inside code fences
 * never count; without a movers heading everything is `before`.
 */
export function splitAtMovers(md: string): MoversSplit {
  const lines = linesOf(md);
  const fenced = fencedLines(lines);
  for (let i = 0; i < lines.length; i++) {
    const m = fenced[i] ? null : MOVERS.exec(lines[i]);
    if (m) return { before: joined(lines.slice(0, i)), heading: m[1], after: joined(lines.slice(i + 1)) };
  }
  return { before: md, heading: null, after: "" };
}

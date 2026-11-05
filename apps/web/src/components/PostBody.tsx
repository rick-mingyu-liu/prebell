import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

/** A same-site path: one leading "/" (not "//" or "/\", which browsers treat as another host). */
const isSameSite = (href: string) => /^\/(?![/\\])/.test(href);

function components(sources: ReadonlySet<string>): Components {
  return {
    // Only the post's own source URLs and same-site paths become links; any other
    // link (LLM- or web-derived, autolinked, reference-style) renders as its text.
    a({ href, title, children }) {
      if (href && sources.has(href)) {
        return (
          <a href={href} title={title} rel="nofollow noopener" target="_blank">
            {children}
          </a>
        );
      }
      if (href && isSameSite(href)) {
        return (
          <a href={href} title={title}>
            {children}
          </a>
        );
      }
      return <>{children}</>;
    },
    // Posts never carry images; rendering one would fetch (and preload) a foreign URL.
    img() {
      return null;
    },
    table({ children }) {
      return (
        <div className="table-wrap">
          <table className="table">{children}</table>
        </div>
      );
    },
  };
}

/** A hast node, as far as `colorSigned` needs one. */
interface HastNode {
  type: string;
  tagName?: string;
  value?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
}

/** An explicitly signed percentage ("+1.2%", "-1.2%", "−1.2%"); not the "-7%" of "5-7%". */
const SIGNED_PCT = /(?<![\w.,%+\-−])([+\-−])(\d[\d,]*(?:\.\d+)?)%/g;

/** A text node split so each signed percentage sits in a `.num` span colored by its own sign. */
function splitSigned(value: string): HastNode[] {
  const out: HastNode[] = [];
  let last = 0;
  for (const m of value.matchAll(SIGNED_PCT)) {
    // A value that rounds to 0.00 is flat, whatever its sign.
    const dir = Math.abs(Number(m[2].replace(/,/g, ""))) < 0.005 ? "flat" : m[1] === "+" ? "up" : "down";
    if (m.index > last) out.push({ type: "text", value: value.slice(last, m.index) });
    out.push({
      type: "element",
      tagName: "span",
      properties: { className: ["num", dir] },
      children: [{ type: "text", value: m[0] }],
    });
    last = m.index + m[0].length;
  }
  if (last === 0) return [{ type: "text", value }];
  if (last < value.length) out.push({ type: "text", value: value.slice(last) });
  return out;
}

/** Elements whose text is never colored: code, and links (a source title or URL is quoted, not a figure of ours). */
const NO_COLOR = new Set(["a", "code", "pre"]);

function colorSignedIn(nodes: HastNode[]): HastNode[] {
  return nodes.flatMap((n) => {
    if (n.type === "text") return splitSigned(n.value ?? "");
    if (n.children && !NO_COLOR.has(n.tagName ?? "")) n.children = colorSignedIn(n.children);
    return [n];
  });
}

/**
 * Rehype plugin: only text outside links and code changes (into text and spans), so links, images
 * and raw HTML are untouched. It runs before `components`, while every link is still an `a`.
 */
const rehypeColorSigned = () => (tree: HastNode) => {
  tree.children = colorSignedIn(tree.children ?? []);
};

/**
 * Renders a post's markdown on the server. Raw HTML is not rendered (no rehype-raw),
 * the default urlTransform drops unsafe URLs such as `javascript:`, links are limited
 * to `sources` and same-site paths, and images are dropped. `colorSigned` colors explicitly
 * signed percentages green or red; unsigned ones stay plain.
 */
export default function PostBody({
  markdown,
  sources,
  colorSigned = false,
}: {
  markdown: string;
  sources: readonly string[];
  colorSigned?: boolean;
}) {
  return (
    <div className="post-body">
      <Markdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={colorSigned ? [rehypeColorSigned] : []}
        components={components(new Set(sources))}
      >
        {markdown}
      </Markdown>
    </div>
  );
}

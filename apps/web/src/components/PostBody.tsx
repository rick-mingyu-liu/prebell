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
          <table>{children}</table>
        </div>
      );
    },
  };
}

/**
 * Renders a post's markdown on the server. Raw HTML is not rendered (no rehype-raw),
 * the default urlTransform drops unsafe URLs such as `javascript:`, links are limited
 * to `sources` and same-site paths, and images are dropped.
 */
export default function PostBody({ markdown, sources }: { markdown: string; sources: readonly string[] }) {
  return (
    <div className="post-body">
      <Markdown remarkPlugins={[remarkGfm]} components={components(new Set(sources))}>
        {markdown}
      </Markdown>
    </div>
  );
}

import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

const isExternal = (href: string | undefined) => !!href && /^https?:\/\//i.test(href);

const components: Components = {
  a({ href, title, children }) {
    return isExternal(href) ? (
      <a href={href} title={title} rel="nofollow noopener" target="_blank">
        {children}
      </a>
    ) : (
      <a href={href} title={title}>
        {children}
      </a>
    );
  },
  table({ children }) {
    return (
      <div className="table-wrap">
        <table>{children}</table>
      </div>
    );
  },
};

/**
 * Renders a post's markdown on the server. Raw HTML is not rendered (no rehype-raw),
 * and the default urlTransform drops unsafe URLs such as `javascript:`.
 */
export default function PostBody({ markdown }: { markdown: string }) {
  return (
    <div className="post-body">
      <Markdown remarkPlugins={[remarkGfm]} components={components}>
        {markdown}
      </Markdown>
    </div>
  );
}

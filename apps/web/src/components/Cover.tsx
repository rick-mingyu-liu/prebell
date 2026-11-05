import { companyLogo } from "../lib/logos";

export interface CoverProps {
  /** R2 key of the graphic (`covers/<slug>.svg`; older posts may hold a `.png` or `.jpg`), or null for the flat light cover. */
  coverKey: string | null;
  /** The two bold lines, or null for a tag-only cover. An empty first line counts as null. */
  lines: [string, string] | null;
  tag: { kind: "logo"; ticker: string } | { kind: "label"; text: string };
  size: "lead" | "article" | "small";
  /** Accessible name for the whole cover; the graphic itself is decorative. */
  alt: string;
}

/** A cover composed in HTML/CSS on paper: the chart graphic at the bottom, then the tag, two ink lines and the amber bar top-left. */
export default function Cover({ coverKey, lines, tag, size, alt }: CoverProps) {
  const logo = tag.kind === "logo" ? companyLogo(tag.ticker) : null;
  const stat = lines && lines[0] !== "" ? lines : null;
  // The lead and article covers sit above the fold, so their graphic loads at once; list covers wait.
  const eager = size !== "small";
  return (
    <figure className={`cover cover-${size}${coverKey ? "" : " cover-flat"}`} aria-label={alt || undefined}>
      {coverKey && (
        <img
          className="cover-bg"
          src={`/${coverKey}`}
          alt=""
          loading={eager ? "eager" : "lazy"}
          fetchPriority={eager ? "high" : undefined}
        />
      )}
      <div className="cover-in">
        <span className="cover-tag">
          {tag.kind === "label" ? tag.text : logo ? <img src={logo} alt={tag.ticker} loading="lazy" /> : tag.ticker}
        </span>
        {stat && (
          <>
            <p className="cover-stat">
              {stat[0]}
              {stat[1] !== "" && (
                <>
                  <br />
                  {stat[1]}
                </>
              )}
            </p>
            <span className="cover-bar"></span>
          </>
        )}
      </div>
    </figure>
  );
}

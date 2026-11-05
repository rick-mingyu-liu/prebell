import { companyLogo } from "../lib/logos";

export interface CoverProps {
  /** R2 key of the scene (`covers/<slug>.png`), or null for the flat near-black cover. */
  coverKey: string | null;
  /** The two bold lines, or null for a tag-only cover. An empty first line counts as null. */
  lines: [string, string] | null;
  tag: { kind: "logo"; ticker: string } | { kind: "label"; text: string };
  size: "lead" | "article" | "small";
  /** Accessible name for the whole cover; the scene itself is decorative. */
  alt: string;
}

/** A 16:9 cover composed in HTML/CSS: dimmed scene, left shade, white tag, two lines and the amber bar. */
export default function Cover({ coverKey, lines, tag, size, alt }: CoverProps) {
  const logo = tag.kind === "logo" ? companyLogo(tag.ticker) : null;
  const stat = lines && lines[0] !== "" ? lines : null;
  return (
    <figure className={`cover cover-${size}`} aria-label={alt || undefined}>
      {coverKey && <img className="cover-bg" src={`/${coverKey}`} alt="" loading="lazy" />}
      <div className="cover-shade"></div>
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

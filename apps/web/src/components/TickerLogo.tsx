import { companyLogo } from "../lib/logos";

/**
 * The company's logo tile beside its ticker. Decorative (the ticker is the text), so alt is empty.
 * Without a logo it renders nothing, or with `placeholder` a lettered tile so rows stay aligned.
 */
export default function TickerLogo({ ticker, placeholder = false }: { ticker: string; placeholder?: boolean }) {
  const src = companyLogo(ticker);
  if (!src)
    return placeholder ? (
      <span className="tlogo tlogo-none" aria-hidden="true">
        {ticker.charAt(0)}
      </span>
    ) : null;
  // A static SVG or PNG tile from /public: nothing for the image optimizer to do, and it stays off the Worker.
  // eslint-disable-next-line @next/next/no-img-element
  return <img className="tlogo" src={src} alt="" width={18} height={18} loading="lazy" decoding="async" />;
}

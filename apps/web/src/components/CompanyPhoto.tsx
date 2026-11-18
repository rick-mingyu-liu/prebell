import type { Locale } from "../i18n/routing";
import type { CompanyPhoto as Photo } from "../lib/photos";
import { translator } from "../lib/translator";

/** A portrait or square photo is narrowed so it is never taller than this (CSS px). */
const MAX_HEIGHT = 520;

/**
 * A company photo from Wikimedia Commons, served from /photos: the image, then the caption and the credit
 * ("Photo: author / Wikimedia Commons · licence · resized · Source"). The credit reads as plain text where
 * Commons is out of reach; the author links to the file's Commons page, a CC licence to its deed, and "Source"
 * to the photo's row on the site's own photo credits page, which lists the rest as text.
 * `eager` is for a photo above the fold (the ticker page); an article's photo, under the lede, loads lazily.
 */
export default function CompanyPhoto({ photo, locale, eager = false }: { photo: Photo; locale: Locale; eager?: boolean }) {
  const t = translator(locale);
  const licence = photo.licence === "Public domain" ? t("photo.publicDomain") : photo.licence;
  // Narrow enough that the height stays within MAX_HEIGHT; a landscape photo fills the column.
  const maxWidth = Math.round((MAX_HEIGHT * photo.width) / photo.height);
  return (
    <figure className="photo">
      {/* A static same-site file, already sized; next/image would add nothing here. */}
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img
        src={photo.src}
        alt={photo.caption}
        width={photo.width}
        height={photo.height}
        loading={eager ? "eager" : "lazy"}
        decoding="async"
        style={maxWidth < 680 ? { maxWidth } : undefined}
      />
      <figcaption>
        {photo.caption}{" "}
        <span className="photo-credit">
          {t("photo.by")}
          <a href={photo.page} rel="noopener">
            {photo.author}
          </a>{" "}
          / Wikimedia Commons ·{" "}
          {photo.licenceUrl ? (
            <a href={photo.licenceUrl} rel="license noopener">
              {licence}
            </a>
          ) : (
            licence
          )}{" "}
          · {t("photo.resized")} · <a href={`/${locale}/photo-credits#${photo.ticker}`}>{t("photo.source")}</a>
        </span>
      </figcaption>
    </figure>
  );
}

import type { Locale } from "../i18n/routing";
import type { CompanyPhoto as Photo } from "../lib/photos";
import { translator } from "../lib/translator";

/** A portrait or square photo is narrowed so it is never taller than this (CSS px). */
const MAX_HEIGHT = 520;

/**
 * A company photo from Wikimedia Commons, served from /photos: the image, then the caption and the credit
 * as plain text (Commons is out of reach in some places, so the credit reads without its links). The author
 * links to the file's Commons page and a CC licence to its deed, as the licences ask.
 */
export default function CompanyPhoto({ photo, locale }: { photo: Photo; locale: Locale }) {
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
        loading="lazy"
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
          )}
        </span>
      </figcaption>
    </figure>
  );
}

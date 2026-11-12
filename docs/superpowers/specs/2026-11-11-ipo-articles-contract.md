# IPO write-ups: the data contract

Date: 2026-11-11 (revised the same day: the derived offering size's inputs and definition, and the model never writing it). Design: `2026-11-11-ipo-articles-design.md`. IPO tables: `2026-11-11-ipo-watch-contract.md`. Seed for local preview: `db/seed-ipo-articles-preview.sql` (one published `range_set` write-up for Anthropic in both languages; apply after `db/seed-ipo-preview.sql`), with its facts object at `db/preview-facts/ipo/anthropic/range_set-0001628280-26-072484.json` (loaded into local R2 by `db/load-preview-facts.sh`).

This is what the site (`apps/web`) reads for the IPO write-ups. The generator is the only writer. No migration: a write-up is an ordinary post (`runs`, `posts`, `post_translations`, `sources`, and one facts object in R2). If this contract has to change, the builder says so in a message first.

Dates are `YYYY-MM-DD` (ET calendar days) unless named `*_at`, which are UTC RFC 3339 with `Z`. Money is US dollars as plain numbers.

## Milestones

| `milestone` | Proven by | `key` | `as_of` (and `posts.trading_date`) |
|---|---|---|---|
| `filed` | the IPO's first public registration statement (`S-1` or `F-1`; a `DRS` never counts) | its accession | its filing date |
| `range_set` | the first read registration cover with a price range | its accession | its filing date |
| `range_changed` | a later read registration cover whose price range or share count differs from the previous range milestone's | its accession | its filing date |
| `priced` | the first read final prospectus (`424B4` or `424B1`) with the offering price | its accession | its filing date |
| `listed` | the first stored trading session (`ipo_trading`) | the session date | the session date |
| `withdrawn` | an `RW` filed after the last offering filing, before any listing | its accession | its filing date |

Only the latest milestone an IPO has reached is ever written up; one it has moved past without a write-up is not written later. A milestone more than 7 days old (ET days from `as_of`) when found is skipped with a `warn` alert.

## Rows

### `runs`

| Column | Value |
|---|---|
| `run_key` | `ipo:<ipo_slug>:<milestone>:<key>`, for example `ipo:anthropic:range_set:0001628280-26-072484`, `ipo:anthropic:listed:2026-11-19`. |
| `content_type` | `ipo` |
| `trading_date` | `as_of` |
| `status` | `pending`, `building`, `published`, `skipped` (too late), `failed` (gave up). |

### `posts`

| Column | Value |
|---|---|
| `run_key` | as `runs.run_key` |
| `content_type` | `ipo` |
| `template_id` | `ipo-<milestone>`, for example `ipo-range_set` |
| `trading_date` | `as_of`: the filing date (the session date for `listed`) |
| `slug` | `<ipo_slug>-ipo-<words>-<as_of>`, words below; if another run already owns it, `-<last 6 of the key without dashes>` is appended. |
| `facts_r2_key` | `ipo/<ipo_slug>/<milestone>-<key>.json`, for example `ipo/anthropic/range_set-0001628280-26-072484.json` |
| `published_at` | set when the post is written |
| `cover_key` | NULL in v1 (no cover image; the site's flat tile with `cover_line1`/`cover_line2`) |
| `revised_at` | NULL in v1 |

Slug words: `filed` → `filing`, `range_set` → `price-range`, `range_changed` → `new-price-range`, `priced` → `priced`, `listed` → `first-trading-day`, `withdrawn` → `withdrawn`. Example: `anthropic-ipo-price-range-2026-11-10`.

A site finds an IPO's write-ups by run-key range, newest first:

```sql
SELECT p.slug, p.template_id, p.trading_date, p.published_at, t.title, t.summary
FROM posts p JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
WHERE p.content_type = 'ipo' AND p.run_key >= 'ipo:' || ? || ':' AND p.run_key < 'ipo:' || ? || ';'
  AND t.status = 'published'
ORDER BY p.trading_date DESC, p.published_at DESC, p.id DESC;
```

### `post_translations`

Both `en` and `zh` are written `published` in one D1 batch (one transaction), so a write-up is never live in one language only. There is no `pending` IPO translation.

| Column | Value |
|---|---|
| `title` | One line. States the milestone with a figure from the filing (for `range_set` both ends of the range). |
| `summary` | One line. |
| `body_md` | The whole article, rendered by code (shape below). |
| `cover_line1` | EN `Anthropic IPO`; ZH `Anthropic IPO` (`name_zh` when set). |
| `cover_line2` | The milestone with its figure, by code: EN `Price range $70.00–$78.00`; ZH `价格区间 70.00–78.00美元`. Others: `Filed for IPO` / `递交IPO申请`, `New price range $X–$Y` / `新价格区间 X–Y美元`, `Priced at $X` / `定价 X美元`, `First day close $X` / `首日收盘 X美元`, `IPO withdrawn` / `撤回IPO申请`. |

### `sources`

One row per link the article cites, in order: the milestone filing's index page (`title` like `Anthropic S-1/A filed 2026-11-10`, `quote` empty), the previous filing for `range_changed`, the Yahoo quote page for `listed`, then each quoted news item (`title` = outlet, `quote` = the item's exact quote).

## `body_md`

Markdown like the earnings write-up. The model writes only the title, the summary and the two prose sections; everything else is rendered by code from the facts. News items reach the article only through the code-rendered "What has been reported" section, word for word. In Chinese the quotes stay in English.

```
# <title>

<summary>

**Anthropic (ANT)** · Nasdaq Global Select Market · Form S-1/A filed Nov 10, 2026

| From the filing | |
|---|---:|
| Price range per share | $70.00–$78.00 |
| Shares offered | 825,000,000 |
| Sold by the company | 801,351,354 |
| Sold by selling stockholders | 23,648,646 |
| Underwriters' option | 123,750,000 shares |
| Offering size at $70.00 (825,000,000 shares × $70.00) | $57.75 billion |
| Offering size at $78.00 (825,000,000 shares × $78.00) | $64.35 billion |
| Exchange and symbol | Nasdaq Global Select Market: ANT |

Offering size is computed by PreBell from the cover: shares offered (including shares sold by selling stockholders, excluding the underwriters' option) × the price per share, gross, before underwriting discounts. It is not the company's proceeds and not a valuation.

## What happened

<model prose>

## What the filing says

<model prose>

## What has been reported

- Reuters, Nov 10, 2026: "<exact quote>" ([article](https://...))
- Jane Doe, an analyst at Example Securities, in Sample Times, Nov 9, 2026: "<exact quote>" ([article](https://...))

## What comes next

<one fixed paragraph per milestone, by code>

## Sources

- [Form S-1/A filed Nov 10, 2026 (SEC EDGAR)](https://www.sec.gov/...)
- [Reuters, Nov 10, 2026](https://...)

This IPO write-up is generated automatically from SEC filings and public news reports, for information only. It is not investment advice.
```

Chinese headings: `发生了什么`, `文件披露了什么`, `已有报道`, `下一步`, `来源`; table header `招股文件数据`; offering-size rows like `按每股70.00美元计算的发行规模（825,000,000股×70.00美元）`, with the definition under the table. The "What has been reported" section is left out when no item is stored. Table rows are shown only for facts the filing has; the derived rows only when the filing has the shares and the price.

## The facts object (R2, `facts_r2_key`)

JSON, written once before the post. Every field is present; a field without data is `null` (objects) or `[]` (lists).

| Field | Type | Meaning |
|---|---|---|
| `schema` | string | `ipo-article/v1` |
| `ipo_slug` | string | `anthropic` (the IPO page is `/[locale]/ipo/<ipo_slug>`) |
| `milestone` | string | above |
| `key` | string | above |
| `run_key` | string | above |
| `name` | string | `Anthropic` |
| `name_zh` | string or null | |
| `cik` | number | `2130357` |
| `symbol` | string or null | from the cover facts (`ANT`) |
| `exchange` | string or null | from the cover facts |
| `as_of` | string | the filing date, or the session date for `listed` |
| `filing` | object or null | The filing whose cover facts are in `cover`: `accession`, `form`, `filing_date`, `index_url`, `doc_url` (string or null). For `listed` the IPO's facts source at the time; for `withdrawn` the `RW`. |
| `cover` | list | `ipo_facts` rows of `filing`: `kind`, `value_num` (number or null), `value_text` (string or null), `quote` (the cover's sentence). Kinds as in the IPO watch contract. |
| `previous` | object or null | `range_changed` only: the previous range milestone's `accession`, `form`, `filing_date`, `index_url`, `price_low`, `price_high`, `shares_offered` (numbers or null). |
| `first_range` | boolean | `range_set` only: true when every earlier registration statement of the IPO was read and none stated a range, so this is provably the first range; false otherwise (Anthropic: false, its S-1 and first S-1/A were never read). The write-up calls the range the first only when true. |
| `derived` | list | The only figures PreBell computes, by code (the model is never given them and may not write them): `kind` (`offering_at_low`, `offering_at_high`, `offering_at_price`), `value_usd`, its two inputs `shares` (the cover's `shares_offered`: sold by the company and by selling stockholders, excluding the underwriters' option) and `price` (the range's low or high end, or the offering price once priced), and `definition` (the sentence below). Nothing else is ever multiplied, never shares outstanding. Show it only with both inputs and as the gross offering size: never as the company's proceeds, what it raises, or a valuation. `definition`: "Gross offering size at this price: shares offered on the cover (sold by the company and by selling stockholders, excluding the underwriters' option) x the price per share, before underwriting discounts. Not the company's proceeds and not a valuation." |
| `session` | object or null | `listed` only: the first session's `session_date`, `symbol`, `open` (number or null), `close`, `volume` (number or null), `ipo_price` (number or null), `change_pct` (number or null), `source_url`, as stored in `ipo_trading`. |
| `timeline` | list | The IPO's filings, newest first: `accession`, `form`, `filing_date`, `index_url`. |
| `items` | list | The news items quoted in the article, in the order shown (newest first, at most 6): `kind`, `outlet`, `url`, `published_on`, `quote`, `speaker` (string or null), `value_usd` (number or null), `value_date` (string or null). |
| `generated_at` | string | UTC time the bundle was gathered. |

Example (Anthropic, `range_set`, the real cover facts of the S-1/A of 2026-11-10; no news items stored yet):

```json
{
  "schema": "ipo-article/v1",
  "ipo_slug": "anthropic",
  "milestone": "range_set",
  "key": "0001628280-26-072484",
  "run_key": "ipo:anthropic:range_set:0001628280-26-072484",
  "name": "Anthropic",
  "name_zh": null,
  "cik": 2130357,
  "symbol": "ANT",
  "exchange": "Nasdaq Global Select Market",
  "as_of": "2026-11-10",
  "filing": {
    "accession": "0001628280-26-072484",
    "form": "S-1/A",
    "filing_date": "2026-11-10",
    "index_url": "https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/0001628280-26-072484-index.htm",
    "doc_url": "https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/anthropicpbc-sx1ax2.htm"
  },
  "cover": [
    {"kind": "price_low", "value_num": 70.0, "value_text": null, "quote": "It is currently estimated that the initial public offering price per share will be between $70.00 and $78.00."},
    {"kind": "price_high", "value_num": 78.0, "value_text": null, "quote": "It is currently estimated that the initial public offering price per share will be between $70.00 and $78.00."},
    {"kind": "shares_offered", "value_num": 825000000.0, "value_text": null, "quote": "Class A Common Stock 825,000,000 Shares"}
  ],
  "previous": null,
  "first_range": false,
  "derived": [
    {"kind": "offering_at_low", "value_usd": 57750000000.0, "shares": 825000000.0, "price": 70.0, "definition": "Gross offering size at this price: ..."},
    {"kind": "offering_at_high", "value_usd": 64350000000.0, "shares": 825000000.0, "price": 78.0, "definition": "Gross offering size at this price: ..."}
  ],
  "session": null,
  "timeline": [
    {"accession": "0001628280-26-072484", "form": "S-1/A", "filing_date": "2026-11-10", "index_url": "https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/0001628280-26-072484-index.htm"}
  ],
  "items": [],
  "generated_at": "2026-11-11T22:00:00Z"
}
```

(The example shortens `cover` to three of its nine rows, `timeline` to one of its nine and each `definition` to its first words; the stored object has them all, in the order `ipo_facts` and `ipo_filings` give.)

## Text the site must treat as untrusted

`title`, `summary` and `body_md` are model-written prose checked by code, and quotes from filings and news articles: render them as Markdown with raw HTML disabled, as for other posts. Links in `body_md` are only SEC, allowlisted outlets and Yahoo.

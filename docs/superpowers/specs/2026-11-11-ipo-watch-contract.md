# IPO watch: the data contract

Date: 2026-11-11. Design: `2026-11-11-ipo-watch-design.md`. Migration: `migrations/0009_ipo_watch.sql`. Seed for local preview: `db/seed-ipo-preview.sql`.

This is what the site (`apps/web`) reads. The generator is the only writer. Everything lives in D1; there is no R2 bundle for IPOs (the facts are small, and one indexed read per table is cheaper than an R2 object). If this contract has to change, the builder says so in a message first.

Dates are `YYYY-MM-DD` (ET calendar days) unless named `*_at`, which are UTC RFC 3339 with `Z`. Money is US dollars as plain numbers (`70`, not `"$70.00"`); the site formats it.

## Tables

### `ipo_watch`: one row per tracked IPO

| Column | Type | Null | Meaning |
|---|---|---|---|
| `slug` | TEXT PK | no | URL segment: `/[locale]/ipo/[slug]`. Lowercase `a-z0-9-`. |
| `cik` | INTEGER | no | SEC CIK. |
| `name` | TEXT | no | Display name from the hand-kept list (`Anthropic`). |
| `name_zh` | TEXT | yes | Chinese display name; NULL means use `name` on Chinese pages. |
| `status` | TEXT | no | `filed`, `range_set`, `priced`, `listed`, `withdrawn` (below). |
| `exchange` | TEXT | yes | As the filing names it (`Nasdaq Global Select Market`). NULL until a cover states it. |
| `symbol` | TEXT | yes | Ticker the filing names (`ANT`). NULL until a cover states it. |
| `facts_accession` | TEXT | yes | The filing whose cover facts are current: join `ipo_facts.accession` and `ipo_filings.accession`. NULL until a cover was read. |
| `first_trade_date` | TEXT | yes | The ET day shown as the first trading day. NULL = "date not set". |
| `date_status` | TEXT | yes | `reported`, `expected` or `confirmed`; NULL exactly when `first_trade_date` is NULL. |
| `date_source` | TEXT | yes | `news`, `filing` or `trading`; NULL with the date. |
| `date_source_name` | TEXT | yes | Who says so: an outlet name (`Reuters`), a filing label (`424B4`), or `Yahoo Finance`. |
| `date_source_url` | TEXT | yes | The article, the filing's index page, or the Yahoo quote page. |
| `date_quote` | TEXT | yes | The sentence the date comes from (an article's or filing's own words); NULL for `trading`. |
| `date_as_of` | TEXT | yes | The day of that source: the article's publication day, the filing date, or the session. |
| `checked_at`, `updated_at` | TEXT | no | Bookkeeping (UTC). |

**Status** (set by code from the forms EDGAR lists, never by a model):

| `status` | Means | Page shows |
|---|---|---|
| `filed` | A registration statement (S-1/F-1, or only a DRS so far) with no price range on its cover. | "Filed"; offering facts as stated, no price. |
| `range_set` | The latest registration statement's cover states a price range. | "Price range set"; shares and the range. |
| `priced` | A final prospectus (424B4 or 424B1) is on file. | "Priced"; the final price (`price_final`). |
| `listed` | Yahoo reported a regular-session close for the symbol (it traded). | "Listed"; the after-listing numbers. |
| `withdrawn` | An `RW` (withdrawal) filed after the last registration statement or prospectus. | "Withdrawn". Hidden from the calendar and the IPO watch list; the page still renders. |

**Date status** (the calendar's dashed/solid treatment):

| `date_status` | `date_source` | Means | Calendar |
|---|---|---|---|
| `reported` | `news` | The most recent date an `expected_date` news item gave (the article said trading would begin on that day). PreBell's wording: "Expected Nov 16, as reported by {outlet} on {date_as_of}". | dashed (unconfirmed) |
| `expected` | `filing` | The next trading day after the pricing date stated on the final prospectus ("Prospectus dated ..."), or after the SEC's `EFFECT` notice when an `8-A12B` is also on file. A rule, not a statement: "Expected Nov 13, the trading day after pricing ({source} filed {date_as_of})". | dashed (unconfirmed) |
| `confirmed` | `filing` or `trading` | A filing states the first trading day in its own words (`date_quote`), or Yahoo shows the symbol's first regular session on that day. | solid |

A reported date that has passed (before yesterday, ET) without a listing is dropped: the row goes back to `first_trade_date` NULL ("date not set"). The item itself stays in `ipo_items`.

### `ipo_filings`: the timeline

| Column | Type | Null | Meaning |
|---|---|---|---|
| `accession` | TEXT PK | no | `0001628280-26-072484`. |
| `slug` | TEXT | no | The IPO. |
| `form` | TEXT | no | EDGAR form code (glossary below). |
| `filing_date` | TEXT | no | EDGAR filing date. |
| `index_url` | TEXT | no | The filing's index page on sec.gov: the link to show. |
| `doc_url` | TEXT | yes | The primary document on sec.gov, when EDGAR names one. |
| `read_state` | TEXT | no | `none` (timeline only), `pending`, `read`, `failed`. The site can ignore it. |
| `attempts` | INTEGER | no | Cover reads tried. |
| `first_seen_at` | TEXT | no | Bookkeeping. |

Timeline: `SELECT form, filing_date, index_url FROM ipo_filings WHERE slug = ? ORDER BY filing_date DESC, accession DESC`. Forms kept: `DRS`, `DRS/A`, `S-1`, `S-1/A`, `F-1`, `F-1/A`, `424B1`, `424B4`, `8-A12B`, `EFFECT`, `CERT`, `RW`.

Form glossary (EN / ZH), for the timeline:

| Form | English | 中文 |
|---|---|---|
| `DRS` | Draft registration statement (confidential) | 保密提交的注册草案 |
| `DRS/A` | Amended draft registration statement | 注册草案修订 |
| `S-1` | Registration statement (S-1) | 注册声明（S-1） |
| `S-1/A` | Amended registration statement (S-1/A) | 注册声明修订（S-1/A） |
| `F-1` | Registration statement (F-1) | 注册声明（F-1） |
| `F-1/A` | Amended registration statement (F-1/A) | 注册声明修订（F-1/A） |
| `424B1`, `424B4` | Final prospectus | 最终招股说明书 |
| `8-A12B` | Exchange registration (8-A) | 交易所注册（8-A） |
| `EFFECT` | Declared effective by the SEC | SEC 宣布注册生效 |
| `CERT` | Exchange certified the listing | 交易所批准上市 |
| `RW` | Registration withdrawn | 撤回注册 |

The same table is `ipo::FORMS` in `crates/core/src/ipo/mod.rs`.

### `ipo_facts`: the offering, from the filing cover

| Column | Type | Null | Meaning |
|---|---|---|---|
| `accession` | TEXT | no | The filing (PK with `kind`). |
| `kind` | TEXT | no | Below. |
| `slug` | TEXT | no | The IPO. |
| `value_num` | REAL | yes | Numeric kinds. |
| `value_text` | TEXT | yes | Text and date kinds. |
| `quote` | TEXT | no | The cover's own words the value was read from; code checked it is in the document and that the number is in it. |

Read the current facts with `SELECT kind, value_num, value_text, quote FROM ipo_facts WHERE accession = ?` (`ipo_watch.facts_accession`), and label them "from the {form} filed {filing_date}" with that filing's `index_url`. Any kind may be missing; show what is there.

| `kind` | Value | Example (Anthropic, S-1/A of 2026-11-10) |
|---|---|---|
| `exchange` | `value_text` | `Nasdaq Global Select Market` |
| `symbol` | `value_text` | `ANT` |
| `shares_offered` | `value_num` | `825000000` (the cover's total) |
| `shares_company` | `value_num` | `801351354` (sold by the company) |
| `shares_selling` | `value_num` | `23648646` (sold by selling stockholders) |
| `option_shares` | `value_num` | `123750000` (the underwriters' option to buy more) |
| `price_low`, `price_high` | `value_num` | `70`, `78` (both share one quote) |
| `price_final` | `value_num` | 424B4/424B1 only |
| `cover_date` | `value_text` | `2026-11-10` ("Subject to Completion. Dated November 10, 2026.") |
| `prospectus_date` | `value_text` | final prospectus only: the pricing date |
| `trading_date` | `value_text` | only when a filing states the first trading day |

There is no deal-size dollar figure in v1: the fee exhibit's "maximum aggregate offering price" includes the underwriters' option and is a fee-calculation number, not the deal size, and PreBell never multiplies shares by price. The page shows shares and the price range (or final price).

### `ipo_items`: as reported

| Column | Type | Null | Meaning |
|---|---|---|---|
| `slug` | TEXT | no | PK part. |
| `kind` | TEXT | no | `valuation`, `expected_date`, `company_move`, `view` (PK part). |
| `url` | TEXT | no | The article (PK part). `http(s)` only. |
| `outlet` | TEXT | no | The outlet's name, checked against the URL's site. |
| `published_on` | TEXT | no | The article's publication day (ET). |
| `quote` | TEXT | no | Word for word from the article. Original language; never translated. |
| `statement_en` | TEXT | yes | One English sentence written from the quote; every number in it is in the quote. NULL for `view`. |
| `statement_zh` | TEXT | yes | The Chinese sentence, number-checked; NULL for `view`, or when the Chinese draft failed (show `statement_en` then, or the quote). |
| `speaker` | TEXT | yes | `view` only: who said it, as the article names them (`Jane Doe, an analyst at Example Securities`). |
| `value_usd` | REAL | yes | `valuation` only: the valuation in dollars (`1.4e12`). |
| `value_date` | TEXT | yes | `expected_date` only: the first trading day the article gives. |
| `found_at` | TEXT | no | When PreBell stored it. |

Queries: `SELECT ... FROM ipo_items WHERE slug = ? AND kind = ? ORDER BY published_on DESC, found_at DESC`. Show:

- **Valuation as reported**: every `valuation` item, newest first, each "$1.4 trillion, reported by {outlet} on {published_on}" with the link and the quote available. PreBell prints each figure; it never writes the change between them.
- **Recent moves**: `company_move` items, newest first (at most 8 are kept).
- **What others say**: `view` items, at most the 3 newest, each as "{speaker}, quoted by {outlet} on {published_on}: “{quote}”". No paraphrase, no PreBell sentence. Omit the section when there are none.
- `expected_date` items need not be listed; the date line already uses the newest one. A page may list them as "Date reports".

Kept per IPO: valuation 10, expected_date 5, company_move 8, view 6 (newest by `published_on`); older ones are deleted.

### `ipo_trading`: after listing

| Column | Type | Null | Meaning |
|---|---|---|---|
| `slug`, `session_date` | TEXT | no | PK. |
| `symbol` | TEXT | no | The ticker read. |
| `open` | REAL | yes | Always NULL in v1 (the recap's quote read has no open). Show it only when present. |
| `close` | REAL | no | The session's regular close. |
| `volume` | INTEGER | yes | Shares traded in the session. |
| `ipo_price` | REAL | yes | The final price from the 424B4 (`price_final`), copied when the row was written. |
| `change_pct` | REAL | yes | Code's (close − ipo_price) / ipo_price × 100, two decimals; NULL without `ipo_price`. Always show both inputs beside it. |
| `source_url` | TEXT | no | `https://finance.yahoo.com/quote/{symbol}`; name the source "Yahoo Finance". |
| `fetched_at` | TEXT | no | Bookkeeping. |

First day: the row with the smallest `session_date`; latest: the largest. Rows are written for 30 days after the first session.

## Calendar and the "IPO watch" list

- **On the calendar** for `from..to`: `SELECT slug, name, name_zh, symbol, status, first_trade_date, date_status FROM ipo_watch WHERE first_trade_date BETWEEN ? AND ? AND status <> 'withdrawn'`. Dashed when `date_status` is `reported` or `expected`, solid when `confirmed`. Link to `/[locale]/ipo/[slug]`.
- **IPO watch list**: `SELECT ... FROM ipo_watch WHERE status IN ('filed', 'range_set', 'priced') OR (status = 'listed' AND first_trade_date >= ?)` with `?` = today (ET) minus 30 days, ordered by `first_trade_date IS NULL, first_trade_date, name`. A NULL date reads "date not set" / "日期未定".
- No row exists for a tracked company until its first filing is seen; an unknown slug is a 404.

## Text the site must treat as untrusted

Everything that came from outside: `ipo_items.quote`, `statement_en`, `statement_zh`, `outlet`, `speaker`, `url`; `ipo_facts.quote` and `value_text`; `ipo_watch.date_quote`, `date_source_name`, `date_source_url`, `exchange`, `symbol`; `ipo_filings.index_url`, `doc_url`. Render as plain text (no Markdown, no HTML), and link a URL only when it starts with `https://` or `http://`. Statements are model-written from the quotes and checked by code (quote word for word in the article, numbers in the quote), but they are still third-party text, not PreBell's voice.

## Language

English and Chinese pages read the same rows. Chinese pages use `name_zh` when set, `statement_zh` when set (else the English statement), the form glossary's Chinese, and the quotes as they are (original language).

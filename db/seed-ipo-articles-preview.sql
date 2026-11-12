-- IPO write-up preview for apps/web (local D1 only). Re-runnable. Contract:
-- docs/superpowers/specs/2026-11-11-ipo-articles-contract.md.
-- Apply after db/seed-ipo-preview.sql: cd crates/generator && pnpm exec wrangler d1 execute prebell --local --file ../../db/seed-ipo-articles-preview.sql
-- Facts object: bash db/load-preview-facts.sh (local R2, ipo/anthropic/range_set-0001628280-26-072484.json).
--
-- One published range_set write-up for Anthropic, EN and ZH, as the generator renders it
-- (crates/core/src/ipo/writeup.rs, test the_preview_seed_is_the_rendered_write_up keeps them equal).
-- REAL: the filing, its cover facts and figures (the S-1/A of 2026-11-10).
-- FIXTURE (made up): the title, summary and two prose sections (a sample model reply that passes the
-- checks), and every quoted news item, outlet and *.example URL (the ipo_items of db/seed-ipo-preview.sql).

DELETE FROM sources WHERE post_id IN (SELECT id FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484');
DELETE FROM post_translations WHERE post_id IN (SELECT id FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484');
DELETE FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484';
DELETE FROM runs WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484';

INSERT INTO runs (run_key, content_type, trading_date, status, attempts, last_error, created_at, updated_at) VALUES
  ('ipo:anthropic:range_set:0001628280-26-072484', 'ipo', '2026-11-10', 'published', 1, NULL, '2026-11-11T22:00:00Z', '2026-11-11T22:00:00Z');

INSERT INTO posts (run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at) VALUES
  ('ipo:anthropic:range_set:0001628280-26-072484', 'ipo', 'ipo-range_set', '2026-11-10', 'anthropic-ipo-price-range-2026-11-10', 'ipo/anthropic/range_set-0001628280-26-072484.json', '2026-11-11T22:00:00Z');

INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, cover_line1, cover_line2, updated_at)
SELECT id, 'en', 'published',
  'Anthropic sets IPO price range at $70 to $78 a share',
  'Anthropic''s amended S-1 filed Nov. 10, 2026 sets a price range of $70.00 to $78.00 a share for 825,000,000 shares of Class A common stock.',
  '# Anthropic sets IPO price range at $70 to $78 a share

Anthropic''s amended S-1 filed Nov. 10, 2026 sets a price range of $70.00 to $78.00 a share for 825,000,000 shares of Class A common stock.

**Anthropic (ANT)** · Nasdaq Global Select Market · Form S-1/A filed Nov 10, 2026

| From the filing | |
|---|---:|
| Price range per share | $70.00–$78.00 |
| Shares offered | 825,000,000 |
| Sold by the company | 801,351,354 |
| Sold by selling stockholders | 23,648,646 |
| Underwriters'' option | 123,750,000 shares |
| Offering size at $70.00 (825,000,000 shares × $70.00) | $57.75 billion |
| Offering size at $78.00 (825,000,000 shares × $78.00) | $64.35 billion |
| Exchange and symbol | Nasdaq Global Select Market: ANT |

Offering size is computed by PreBell from the cover: shares offered (including shares sold by selling stockholders, excluding the underwriters'' option) × the price per share, gross, before underwriting discounts. It is not the company''s proceeds and not a valuation.

## What happened

Anthropic filed an amended registration statement (Form S-1/A) on Nov. 10, 2026. Its cover sets a price range of $70.00 to $78.00 per share for 825,000,000 shares of Class A common stock.

## What the filing says

Anthropic is selling 801,351,354 of the shares, and selling stockholders are selling 23,648,646. The underwriters have an option to purchase up to 123,750,000 additional shares. Anthropic has applied to list its Class A common stock on the Nasdaq Global Select Market under the symbol ANT.

## What has been reported

- Example News, Nov 10, 2026: "At the top of the range the offering would value Anthropic at roughly $1.4 trillion, the people said." ([article](https://news.example/2026/11/10/anthropic-ipo-pricing))
- Example News, Nov 10, 2026: "Anthropic is expected to begin trading on the Nasdaq on Nov. 16 after pricing its shares on Nov. 13, the people said." ([article](https://news.example/2026/11/10/anthropic-ipo-pricing))
- Jane Doe, an analyst at Example Securities, in Sample Times, Nov 9, 2026: "The office tools show how far they want to push into software that businesses already pay for" ([article](https://sampletimes.example/2026/11/09/anthropic-ipo-analysts))
- Sample Times, Nov 4, 2026: "Anthropic on Tuesday released AI versions of word-processing and presentation software for business customers." ([article](https://sampletimes.example/2026/11/04/anthropic-office-apps))
- Sample Times, Oct 12, 2026: "Bankers have pitched a valuation of about $2 trillion for the listing, according to people familiar with the talks." ([article](https://sampletimes.example/2026/10/12/anthropic-valuation))

## What comes next

The range can still change in a later amendment. The offering price is set at pricing and stated in a final prospectus (Form 424B4), after which the shares begin trading.

## Sources

- [Form S-1/A filed Nov 10, 2026 (SEC EDGAR)](https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/0001628280-26-072484-index.htm)
- [Example News, Nov 10, 2026](https://news.example/2026/11/10/anthropic-ipo-pricing)
- [Sample Times, Nov 9, 2026](https://sampletimes.example/2026/11/09/anthropic-ipo-analysts)
- [Sample Times, Nov 4, 2026](https://sampletimes.example/2026/11/04/anthropic-office-apps)
- [Sample Times, Oct 12, 2026](https://sampletimes.example/2026/10/12/anthropic-valuation)

This IPO write-up is generated automatically from SEC filings and public news reports, for information only. It is not investment advice.
',
  'Anthropic IPO', 'Price range $70.00–$78.00', '2026-11-11T22:00:00Z'
FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484';

INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, cover_line1, cover_line2, updated_at)
SELECT id, 'zh', 'published',
  'Anthropic公布IPO价格区间：每股70至78美元',
  'Anthropic于2026年11月10日提交S-1/A修订文件，将8.25亿股A类普通股的价格区间定为每股70.00至78.00美元。',
  '# Anthropic公布IPO价格区间：每股70至78美元

Anthropic于2026年11月10日提交S-1/A修订文件，将8.25亿股A类普通股的价格区间定为每股70.00至78.00美元。

**Anthropic（ANT）** · Nasdaq Global Select Market · S-1/A 提交于2026年11月10日

| 招股文件数据 | |
|---|---:|
| 每股价格区间 | 70.00–78.00美元 |
| 发行股数 | 825,000,000股 |
| 公司发售 | 801,351,354股 |
| 售股股东发售 | 23,648,646股 |
| 承销商超额配售权 | 123,750,000股 |
| 按每股70.00美元计算的发行规模（825,000,000股×70.00美元） | 577.5亿美元 |
| 按每股78.00美元计算的发行规模（825,000,000股×78.00美元） | 643.5亿美元 |
| 交易所与代码 | Nasdaq Global Select Market：ANT |

发行规模由PreBell根据招股文件封面计算：发行股数（含售股股东发售的股份，不含承销商超额配售权）×每股价格，为扣除承销折扣前的总额，并非公司所得款项，也不是估值。

## 发生了什么

Anthropic于2026年11月10日提交了注册声明修订文件（S-1/A）。文件将价格区间定为每股70.00美元至78.00美元，发行8.25亿股A类普通股。

## 文件披露了什么

其中公司发售801,351,354股，售股股东发售23,648,646股。承销商有权额外购买至多123,750,000股。公司已申请在Nasdaq Global Select Market上市，股票代码为ANT。

## 已有报道

- Example News（2026年11月10日）报道：“At the top of the range the offering would value Anthropic at roughly $1.4 trillion, the people said.”（[原文](https://news.example/2026/11/10/anthropic-ipo-pricing)）
- Example News（2026年11月10日）报道：“Anthropic is expected to begin trading on the Nasdaq on Nov. 16 after pricing its shares on Nov. 13, the people said.”（[原文](https://news.example/2026/11/10/anthropic-ipo-pricing)）
- Jane Doe, an analyst at Example Securities（Sample Times，2026年11月9日）：“The office tools show how far they want to push into software that businesses already pay for”（[原文](https://sampletimes.example/2026/11/09/anthropic-ipo-analysts)）
- Sample Times（2026年11月4日）报道：“Anthropic on Tuesday released AI versions of word-processing and presentation software for business customers.”（[原文](https://sampletimes.example/2026/11/04/anthropic-office-apps)）
- Sample Times（2026年10月12日）报道：“Bankers have pitched a valuation of about $2 trillion for the listing, according to people familiar with the talks.”（[原文](https://sampletimes.example/2026/10/12/anthropic-valuation)）

## 下一步

价格区间仍可能在之后的修订文件中调整。发行价在定价时确定，并在最终招股说明书（424B4表格）中载明，之后股票开始交易。

## 来源

- [S-1/A，提交于2026年11月10日（SEC EDGAR）](https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/0001628280-26-072484-index.htm)
- [Example News，2026年11月10日](https://news.example/2026/11/10/anthropic-ipo-pricing)
- [Sample Times，2026年11月9日](https://sampletimes.example/2026/11/09/anthropic-ipo-analysts)
- [Sample Times，2026年11月4日](https://sampletimes.example/2026/11/04/anthropic-office-apps)
- [Sample Times，2026年10月12日](https://sampletimes.example/2026/10/12/anthropic-valuation)

本文根据美国证监会文件和公开新闻报道自动生成，仅供参考，不构成投资建议。
',
  'Anthropic IPO', '价格区间 70.00–78.00美元', '2026-11-11T22:00:00Z'
FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484';

INSERT INTO sources (post_id, url, title, quote)
SELECT id, 'https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/0001628280-26-072484-index.htm', 'Anthropic S-1/A filed 2026-11-10', '' FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484';
INSERT INTO sources (post_id, url, title, quote)
SELECT id, 'https://news.example/2026/11/10/anthropic-ipo-pricing', 'Example News', 'At the top of the range the offering would value Anthropic at roughly $1.4 trillion, the people said.' FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484';
INSERT INTO sources (post_id, url, title, quote)
SELECT id, 'https://news.example/2026/11/10/anthropic-ipo-pricing', 'Example News', 'Anthropic is expected to begin trading on the Nasdaq on Nov. 16 after pricing its shares on Nov. 13, the people said.' FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484';
INSERT INTO sources (post_id, url, title, quote)
SELECT id, 'https://sampletimes.example/2026/11/09/anthropic-ipo-analysts', 'Sample Times', 'The office tools show how far they want to push into software that businesses already pay for' FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484';
INSERT INTO sources (post_id, url, title, quote)
SELECT id, 'https://sampletimes.example/2026/11/04/anthropic-office-apps', 'Sample Times', 'Anthropic on Tuesday released AI versions of word-processing and presentation software for business customers.' FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484';
INSERT INTO sources (post_id, url, title, quote)
SELECT id, 'https://sampletimes.example/2026/10/12/anthropic-valuation', 'Sample Times', 'Bankers have pitched a valuation of about $2 trillion for the listing, according to people familiar with the talks.' FROM posts WHERE run_key = 'ipo:anthropic:range_set:0001628280-26-072484';

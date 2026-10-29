-- Preview data for apps/web (local D1 only). Re-runnable.
-- Apply: cd crates/generator && pnpm exec wrangler d1 execute prebell --local --file ../../db/seed-preview.sql
--   premarket-2026-10-27: EN published + ZH published (hreflang pair)
--   premarket-2026-10-28: EN published + ZH pending (ZH page shows the pending notice, noindex)

DELETE FROM sources WHERE post_id IN (SELECT id FROM posts WHERE run_key IN ('premarket:2026-10-27','premarket:2026-10-28'));
DELETE FROM post_translations WHERE post_id IN (SELECT id FROM posts WHERE run_key IN ('premarket:2026-10-27','premarket:2026-10-28'));
DELETE FROM posts WHERE run_key IN ('premarket:2026-10-27','premarket:2026-10-28');
DELETE FROM runs WHERE run_key IN ('premarket:2026-10-27','premarket:2026-10-28');

INSERT INTO runs (run_key, content_type, trading_date, status, attempts, last_error, created_at, updated_at) VALUES
  ('premarket:2026-10-27', 'premarket', '2026-10-27', 'published', 1, NULL, '2026-10-27T12:30:00Z', '2026-10-27T12:34:00Z'),
  ('premarket:2026-10-28', 'premarket', '2026-10-28', 'published', 1, NULL, '2026-10-28T12:30:00Z', '2026-10-28T12:36:00Z');

INSERT INTO posts (run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at) VALUES
  ('premarket:2026-10-27', 'premarket', 'premarket:v1', '2026-10-27', 'premarket-2026-10-27', 'facts/premarket/2026-10-27.json', '2026-10-27T12:34:00Z'),
  ('premarket:2026-10-28', 'premarket', 'premarket:v1', '2026-10-28', 'premarket-2026-10-28', 'facts/premarket/2026-10-28.json', '2026-10-28T12:36:00Z');

INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
SELECT id, 'en', 'published',
  'Futures edge up; Apple jumps 6% on services beat',
  'Stock futures are slightly higher ahead of the open as Apple rallies after earnings and Treasury yields ease.',
  '# Futures edge up; Apple jumps 6% on services beat

Stock futures are slightly higher ahead of the open as Apple rallies after earnings and Treasury yields ease.

| Contract | Last | Change |
|---|---:|---:|
| S&P 500 | 5,912.25 | +0.42% |
| Nasdaq 100 | 20,874.50 | +0.71% |
| Dow | 43,105.00 | +0.18% |
| Russell 2000 | 2,241.30 | -0.12% |

| Ticker | Company | Premarket | Change | Why |
|---|---|---:|---:|---|
| AAPL | Apple Inc. | 241.10 | +6.12% | Services revenue beat estimates |
| AMD | Advanced Micro Devices | 158.42 | +4.05% | Raised data-center outlook |
| PYPL | PayPal Holdings | 71.88 | -5.30% | Weaker branded checkout growth |

## Futures

S&P 500 futures are up 0.42% and Nasdaq 100 futures are up 0.71%, led by megacap tech. Russell 2000 futures are slightly lower.

## Biggest premarket movers

Apple is up 6.12% after services revenue beat estimates. AMD is up 4.05% on a raised data-center outlook. PayPal is down 5.30% after branded checkout growth slowed.

## Macro & overnight

The 10-year Treasury yield eased overnight. European stocks are mixed.

## What to watch

Consumer confidence data at 10:00 AM ET, and earnings from Microsoft and Meta after the close.

## Sources

- [Apple beats on services revenue](https://news.example.com/apple-q4)
- [AMD raises data-center outlook](https://news.example.com/amd-outlook)
- [PayPal checkout growth slows](https://news.example.com/paypal-q3)

This brief is generated automatically from public data for information only. It is not investment advice.
', '2026-10-27T12:34:00Z'
FROM posts WHERE run_key = 'premarket:2026-10-27';

INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
SELECT id, 'zh', 'published',
  '期货小幅走高；苹果服务业务超预期大涨6%',
  '开盘前股指期货小幅走高，苹果财报后大涨，美债收益率回落。',
  '# 期货小幅走高；苹果服务业务超预期大涨6%

开盘前股指期货小幅走高，苹果财报后大涨，美债收益率回落。

| 合约 | 最新 | 涨跌 |
|---|---:|---:|
| S&P 500 | 5,912.25 | +0.42% |
| Nasdaq 100 | 20,874.50 | +0.71% |
| Dow | 43,105.00 | +0.18% |
| Russell 2000 | 2,241.30 | -0.12% |

| 代码 | 公司 | 盘前价 | 涨跌 |
|---|---|---:|---:|
| AAPL | Apple Inc. | 241.10 | +6.12% |
| AMD | Advanced Micro Devices | 158.42 | +4.05% |
| PYPL | PayPal Holdings | 71.88 | -5.30% |

## 期货

标普500期货上涨0.42%，纳指100期货上涨0.71%，大型科技股领涨。罗素2000期货小幅走低。

## 盘前异动

苹果服务收入超预期，盘前上涨6.12%。AMD上调数据中心业务展望，上涨4.05%。PayPal品牌结账业务增长放缓，下跌5.30%。

## 宏观与隔夜

10年期美债收益率隔夜回落，欧洲股市涨跌互现。

## 今日关注

美东时间上午10:00公布消费者信心数据；微软和Meta盘后发布财报。

## 来源

- [Apple beats on services revenue](https://news.example.com/apple-q4)
- [AMD raises data-center outlook](https://news.example.com/amd-outlook)
- [PayPal checkout growth slows](https://news.example.com/paypal-q3)

本简报根据公开数据自动生成，仅供参考，不构成投资建议。
', '2026-10-27T12:40:00Z'
FROM posts WHERE run_key = 'premarket:2026-10-27';

INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
SELECT id, 'en', 'published',
  'Futures slip as chipmakers fall; Visa gains after results',
  'Stock futures are lower ahead of the open, weighed down by semiconductor stocks, while Visa rises after earnings.',
  '# Futures slip as chipmakers fall; Visa gains after results

Stock futures are lower ahead of the open, weighed down by semiconductor stocks, while Visa rises after earnings.

| Contract | Last | Change |
|---|---:|---:|
| S&P 500 | 5,889.75 | -0.38% |
| Nasdaq 100 | 20,702.25 | -0.82% |
| Dow | 43,050.00 | -0.13% |
| Russell 2000 | N/A | N/A |

| Ticker | Company | Premarket | Change | Why |
|---|---|---:|---:|---|
| V | Visa Inc. | 312.60 | +3.44% | Payments volume beat estimates |
| NXPI | NXP Semiconductors | 201.15 | -7.81% | Guided fourth-quarter revenue below estimates |

## Futures

S&P 500 futures are down 0.38% and Nasdaq 100 futures are down 0.82% as chip stocks fall. Russell 2000 futures data was not available.

## Biggest premarket movers

Visa is up 3.44% after payments volume beat estimates. NXP Semiconductors is down 7.81% after guiding fourth-quarter revenue below estimates.

## Macro & overnight

Oil prices are little changed. Asian markets closed mostly lower.

## What to watch

The Federal Reserve rate decision at 2:00 PM ET.

## Sources

- [Visa payments volume beats](https://news.example.com/visa-q4)
- [NXP guides below estimates](https://news.example.com/nxp-guide)

This brief is generated automatically from public data for information only. It is not investment advice.
', '2026-10-28T12:36:00Z'
FROM posts WHERE run_key = 'premarket:2026-10-28';

INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
SELECT id, 'zh', 'pending', NULL, NULL, NULL, '2026-10-28T12:36:00Z'
FROM posts WHERE run_key = 'premarket:2026-10-28';

INSERT INTO sources (post_id, url, title, quote)
SELECT id, 'https://news.example.com/apple-q4', 'Apple beats on services revenue', 'Services revenue rose to a record.' FROM posts WHERE run_key = 'premarket:2026-10-27'
UNION ALL SELECT id, 'https://news.example.com/amd-outlook', 'AMD raises data-center outlook', 'AMD raised its data-center outlook.' FROM posts WHERE run_key = 'premarket:2026-10-27'
UNION ALL SELECT id, 'https://news.example.com/paypal-q3', 'PayPal checkout growth slows', 'Branded checkout growth slowed.' FROM posts WHERE run_key = 'premarket:2026-10-27'
UNION ALL SELECT id, 'https://news.example.com/visa-q4', 'Visa payments volume beats', 'Payments volume beat estimates.' FROM posts WHERE run_key = 'premarket:2026-10-28'
UNION ALL SELECT id, 'https://news.example.com/nxp-guide', 'NXP guides below estimates', 'Fourth-quarter guidance was below estimates.' FROM posts WHERE run_key = 'premarket:2026-10-28';

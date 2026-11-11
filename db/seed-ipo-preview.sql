-- IPO watch preview data for apps/web (local D1 only). Re-runnable.
-- Needs migration 0009 locally: cd crates/generator && pnpm exec wrangler d1 migrations apply prebell --local
-- Apply: cd crates/generator && pnpm exec wrangler d1 execute prebell --local --file ../../db/seed-ipo-preview.sql
--
-- Anthropic's row, filings and cover facts are REAL: EDGAR, checked 2026-11-11 (submissions JSON and the
-- 2026-11-10 S-1/A cover, crates/core/tests/fixtures/edgar_ipo_*).
-- FIXTURE (made up): every ipo_items row, every outlet and article URL (*.example), and the two other
-- companies (Northwind Robotics, Harbor Bio) with their filings, facts and trading rows.

DELETE FROM ipo_trading WHERE slug IN ('anthropic', 'northwind-robotics', 'harbor-bio');
DELETE FROM ipo_items WHERE slug IN ('anthropic', 'northwind-robotics', 'harbor-bio');
DELETE FROM ipo_facts WHERE slug IN ('anthropic', 'northwind-robotics', 'harbor-bio');
DELETE FROM ipo_filings WHERE slug IN ('anthropic', 'northwind-robotics', 'harbor-bio');
DELETE FROM ipo_watch WHERE slug IN ('anthropic', 'northwind-robotics', 'harbor-bio');

-- Anthropic: range_set; the date is a (made-up) news report, so dashed.
INSERT INTO ipo_watch (slug, cik, name, name_zh, status, exchange, symbol, facts_accession, first_trade_date,
  date_status, date_source, date_source_name, date_source_url, date_quote, date_as_of, checked_at, updated_at) VALUES
  ('anthropic', 2130357, 'Anthropic', NULL, 'range_set', 'Nasdaq Global Select Market', 'ANT', '0001628280-26-072484',
   '2026-11-16', 'reported', 'news', 'Example News',
   'https://news.example/2026/11/10/anthropic-ipo-pricing',
   'Anthropic is expected to begin trading on the Nasdaq on Nov. 16 after pricing its shares on Nov. 13, the people said.',
   '2026-11-10', '2026-11-11T15:00:00Z', '2026-11-11T15:00:00Z');

INSERT INTO ipo_filings (accession, slug, form, filing_date, index_url, doc_url, read_state, attempts, first_seen_at) VALUES
  ('0001628280-26-072484', 'anthropic', 'S-1/A', '2026-11-10', 'https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/0001628280-26-072484-index.htm', 'https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/anthropicpbc-sx1ax2.htm', 'read', 1, '2026-11-11T15:00:00Z'),
  ('0001628280-26-068676', 'anthropic', 'S-1/A', '2026-11-02', 'https://www.sec.gov/Archives/edgar/data/2130357/000162828026068676/0001628280-26-068676-index.htm', 'https://www.sec.gov/Archives/edgar/data/2130357/000162828026068676/anthropicpbc-sx1ax1.htm', 'none', 0, '2026-11-11T15:00:00Z'),
  ('0001628280-26-067168', 'anthropic', 'S-1', '2026-10-26', 'https://www.sec.gov/Archives/edgar/data/2130357/000162828026067168/0001628280-26-067168-index.htm', 'https://www.sec.gov/Archives/edgar/data/2130357/000162828026067168/anthropicpbc-sx1.htm', 'none', 0, '2026-11-11T15:00:00Z'),
  ('0002130357-26-000014', 'anthropic', 'DRS/A', '2026-10-09', 'https://www.sec.gov/Archives/edgar/data/2130357/000213035726000014/0002130357-26-000014-index.htm', 'https://www.sec.gov/Archives/edgar/data/2130357/000213035726000014/filename1.htm', 'none', 0, '2026-11-11T15:00:00Z'),
  ('0002130357-26-000009', 'anthropic', 'DRS/A', '2026-10-01', 'https://www.sec.gov/Archives/edgar/data/2130357/000213035726000009/0002130357-26-000009-index.htm', 'https://www.sec.gov/Archives/edgar/data/2130357/000213035726000009/filename1.htm', 'none', 0, '2026-11-11T15:00:00Z'),
  ('0001628279-26-001111', 'anthropic', 'DRS/A', '2026-09-04', 'https://www.sec.gov/Archives/edgar/data/2130357/000162827926001111/0001628279-26-001111-index.htm', 'https://www.sec.gov/Archives/edgar/data/2130357/000162827926001111/filename1.htm', 'none', 0, '2026-11-11T15:00:00Z'),
  ('0001628279-26-000993', 'anthropic', 'DRS/A', '2026-08-10', 'https://www.sec.gov/Archives/edgar/data/2130357/000162827926000993/0001628279-26-000993-index.htm', 'https://www.sec.gov/Archives/edgar/data/2130357/000162827926000993/filename1.htm', 'none', 0, '2026-11-11T15:00:00Z'),
  ('0001628279-26-000861', 'anthropic', 'DRS/A', '2026-07-13', 'https://www.sec.gov/Archives/edgar/data/2130357/000162827926000861/0001628279-26-000861-index.htm', 'https://www.sec.gov/Archives/edgar/data/2130357/000162827926000861/filename1.htm', 'none', 0, '2026-11-11T15:00:00Z'),
  ('0002130357-26-000005', 'anthropic', 'DRS', '2026-06-01', 'https://www.sec.gov/Archives/edgar/data/2130357/000213035726000005/0002130357-26-000005-index.htm', 'https://www.sec.gov/Archives/edgar/data/2130357/000213035726000005/filename1.htm', 'none', 0, '2026-11-11T15:00:00Z');

-- Real cover facts of the 2026-11-10 S-1/A, each with its sentence as the cover states it.
INSERT INTO ipo_facts (accession, kind, slug, value_num, value_text, quote) VALUES
  ('0001628280-26-072484', 'cover_date', 'anthropic', NULL, '2026-11-10', 'Subject to Completion. Dated November 10, 2026.'),
  ('0001628280-26-072484', 'shares_offered', 'anthropic', 825000000, NULL, 'Class A Common Stock 825,000,000 Shares'),
  ('0001628280-26-072484', 'shares_company', 'anthropic', 801351354, NULL, 'We are selling 801,351,354 shares of Class A common stock and the selling stockholders identified in this prospectus are selling an additional 23,648,646 shares of Class A common stock.'),
  ('0001628280-26-072484', 'shares_selling', 'anthropic', 23648646, NULL, 'We are selling 801,351,354 shares of Class A common stock and the selling stockholders identified in this prospectus are selling an additional 23,648,646 shares of Class A common stock.'),
  ('0001628280-26-072484', 'price_low', 'anthropic', 70, NULL, 'It is currently estimated that the initial public offering price per share will be between $70.00 and $78.00.'),
  ('0001628280-26-072484', 'price_high', 'anthropic', 78, NULL, 'It is currently estimated that the initial public offering price per share will be between $70.00 and $78.00.'),
  ('0001628280-26-072484', 'exchange', 'anthropic', NULL, 'Nasdaq Global Select Market', 'We have applied to list our Class A common stock on the Nasdaq Global Select Market under the symbol “ANT.”'),
  ('0001628280-26-072484', 'symbol', 'anthropic', NULL, 'ANT', 'We have applied to list our Class A common stock on the Nasdaq Global Select Market under the symbol “ANT.”'),
  ('0001628280-26-072484', 'option_shares', 'anthropic', 123750000, NULL, 'To the extent that the underwriters sell more than 825,000,000 shares of Class A common stock, the underwriters have the option to purchase up to an additional 123,750,000 shares of Class A common stock from Anthropic, PBC at the initial public offering price less the underwriting discount.');

-- FIXTURE: made-up news items for Anthropic (two valuations on different dates, a date report,
-- a company move, a view). Only a company move carries a statement; the others render from
-- value_usd / value_date / speaker and the quote (contract, ipo_items).
INSERT INTO ipo_items (slug, kind, url, outlet, published_on, quote, statement_en, statement_zh, speaker, value_usd, value_date, found_at) VALUES
  ('anthropic', 'valuation', 'https://sampletimes.example/2026/10/12/anthropic-valuation', 'Sample Times', '2026-10-12',
   'Bankers have pitched a valuation of about $2 trillion for the listing, according to people familiar with the talks.',
   NULL, NULL, NULL, 2000000000000, NULL, '2026-11-11T15:00:00Z'),
  ('anthropic', 'valuation', 'https://news.example/2026/11/10/anthropic-ipo-pricing', 'Example News', '2026-11-10',
   'At the top of the range the offering would value Anthropic at roughly $1.4 trillion, the people said.',
   NULL, NULL, NULL, 1400000000000, NULL, '2026-11-11T15:00:00Z'),
  ('anthropic', 'expected_date', 'https://news.example/2026/11/10/anthropic-ipo-pricing', 'Example News', '2026-11-10',
   'Anthropic is expected to begin trading on the Nasdaq on Nov. 16 after pricing its shares on Nov. 13, the people said.',
   NULL, NULL, NULL, NULL, '2026-11-16', '2026-11-11T15:00:00Z'),
  ('anthropic', 'company_move', 'https://sampletimes.example/2026/11/04/anthropic-office-apps', 'Sample Times', '2026-11-04',
   'Anthropic on Tuesday released AI versions of word-processing and presentation software for business customers.',
   'Released AI word-processing and presentation software for business customers.',
   '面向企业客户推出了 AI 文字处理和演示文稿软件。', NULL, NULL, NULL, '2026-11-11T15:00:00Z'),
  ('anthropic', 'view', 'https://sampletimes.example/2026/11/09/anthropic-ipo-analysts', 'Sample Times', '2026-11-09',
   'The office tools show how far they want to push into software that businesses already pay for',
   NULL, NULL, 'Jane Doe, an analyst at Example Securities', NULL, NULL, '2026-11-11T15:00:00Z');

-- FIXTURE: Northwind Robotics, listed (NYSE: NWRB), priced at $24.00, confirmed by trading.
INSERT INTO ipo_watch (slug, cik, name, name_zh, status, exchange, symbol, facts_accession, first_trade_date,
  date_status, date_source, date_source_name, date_source_url, date_quote, date_as_of, checked_at, updated_at) VALUES
  ('northwind-robotics', 9999990001, 'Northwind Robotics', '北风机器人', 'listed', 'New York Stock Exchange', 'NWRB',
   '9999990001-26-000020', '2026-11-05', 'confirmed', 'trading', 'Yahoo Finance',
   'https://finance.yahoo.com/quote/NWRB', NULL, '2026-11-05', '2026-11-11T15:00:00Z', '2026-11-11T15:00:00Z');

INSERT INTO ipo_filings (accession, slug, form, filing_date, index_url, doc_url, read_state, attempts, first_seen_at) VALUES
  ('9999990001-26-000020', 'northwind-robotics', '424B4', '2026-11-05', 'https://www.sec.gov/Archives/edgar/data/9999990001/999999000126000020/9999990001-26-000020-index.htm', NULL, 'read', 1, '2026-11-05T12:00:00Z'),
  ('9999990001-26-000018', 'northwind-robotics', 'EFFECT', '2026-11-04', 'https://www.sec.gov/Archives/edgar/data/9999990001/999999000126000018/9999990001-26-000018-index.htm', NULL, 'none', 0, '2026-11-05T12:00:00Z'),
  ('9999990001-26-000017', 'northwind-robotics', '8-A12B', '2026-11-03', 'https://www.sec.gov/Archives/edgar/data/9999990001/999999000126000017/9999990001-26-000017-index.htm', NULL, 'none', 0, '2026-11-05T12:00:00Z'),
  ('9999990001-26-000012', 'northwind-robotics', 'S-1/A', '2026-10-27', 'https://www.sec.gov/Archives/edgar/data/9999990001/999999000126000012/9999990001-26-000012-index.htm', NULL, 'read', 1, '2026-10-27T12:00:00Z'),
  ('9999990001-26-000004', 'northwind-robotics', 'S-1', '2026-10-06', 'https://www.sec.gov/Archives/edgar/data/9999990001/999999000126000004/9999990001-26-000004-index.htm', NULL, 'none', 0, '2026-10-06T12:00:00Z');

INSERT INTO ipo_facts (accession, kind, slug, value_num, value_text, quote) VALUES
  ('9999990001-26-000020', 'shares_offered', 'northwind-robotics', 20000000, NULL, '20,000,000 Shares Class A Common Stock'),
  ('9999990001-26-000020', 'shares_company', 'northwind-robotics', 20000000, NULL, 'Northwind Robotics, Inc. is offering 20,000,000 shares of its Class A common stock.'),
  ('9999990001-26-000020', 'price_final', 'northwind-robotics', 24, NULL, 'The initial public offering price is $24.00 per share.'),
  ('9999990001-26-000020', 'exchange', 'northwind-robotics', NULL, 'New York Stock Exchange', 'Our Class A common stock has been approved for listing on the New York Stock Exchange under the symbol “NWRB.”'),
  ('9999990001-26-000020', 'symbol', 'northwind-robotics', NULL, 'NWRB', 'Our Class A common stock has been approved for listing on the New York Stock Exchange under the symbol “NWRB.”'),
  ('9999990001-26-000020', 'prospectus_date', 'northwind-robotics', NULL, '2026-11-04', 'Prospectus dated November 4, 2026');

INSERT INTO ipo_trading (slug, session_date, symbol, open, close, volume, ipo_price, change_pct, source_url, fetched_at) VALUES
  ('northwind-robotics', '2026-11-05', 'NWRB', NULL, 31.2, 48210000, 24, 30, 'https://finance.yahoo.com/quote/NWRB', '2026-11-05T21:00:00Z'),
  ('northwind-robotics', '2026-11-10', 'NWRB', NULL, 28.45, 9120000, 24, 18.54, 'https://finance.yahoo.com/quote/NWRB', '2026-11-10T21:00:00Z');

INSERT INTO ipo_items (slug, kind, url, outlet, published_on, quote, statement_en, statement_zh, speaker, value_usd, value_date, found_at) VALUES
  ('northwind-robotics', 'company_move', 'https://news.example/2026/10/30/northwind-warehouse-deal', 'Example News', '2026-10-30',
   'Northwind Robotics signed a deal to supply picking robots to a national grocery chain, the company said.',
   'Signed a deal to supply picking robots to a national grocery chain.',
   '签署协议，向一家全国性连锁超市供应拣货机器人。', NULL, NULL, NULL, '2026-11-01T12:00:00Z');

-- FIXTURE: Harbor Bio, filed, no price range, no date.
INSERT INTO ipo_watch (slug, cik, name, name_zh, status, exchange, symbol, facts_accession, first_trade_date,
  date_status, date_source, date_source_name, date_source_url, date_quote, date_as_of, checked_at, updated_at) VALUES
  ('harbor-bio', 9999990002, 'Harbor Bio', NULL, 'filed', 'Nasdaq Global Market', 'HRBO', '9999990002-26-000003',
   NULL, NULL, NULL, NULL, NULL, NULL, NULL, '2026-11-11T15:00:00Z', '2026-11-11T15:00:00Z');

INSERT INTO ipo_filings (accession, slug, form, filing_date, index_url, doc_url, read_state, attempts, first_seen_at) VALUES
  ('9999990002-26-000003', 'harbor-bio', 'S-1', '2026-11-09', 'https://www.sec.gov/Archives/edgar/data/9999990002/999999000226000003/9999990002-26-000003-index.htm', NULL, 'read', 1, '2026-11-09T14:00:00Z');

INSERT INTO ipo_facts (accession, kind, slug, value_num, value_text, quote) VALUES
  ('9999990002-26-000003', 'exchange', 'harbor-bio', NULL, 'Nasdaq Global Market', 'We have applied to list our common stock on the Nasdaq Global Market under the symbol “HRBO.”'),
  ('9999990002-26-000003', 'symbol', 'harbor-bio', NULL, 'HRBO', 'We have applied to list our common stock on the Nasdaq Global Market under the symbol “HRBO.”'),
  ('9999990002-26-000003', 'cover_date', 'harbor-bio', NULL, '2026-11-09', 'Subject to Completion. Dated November 9, 2026.');

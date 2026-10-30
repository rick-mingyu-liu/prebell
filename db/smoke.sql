INSERT INTO runs (run_key, content_type, trading_date, status, attempts, created_at, updated_at)
VALUES ('premarket-2026-01-05', 'premarket', '2026-01-05', 'published', 1, '2026-01-05T13:00:00Z', '2026-01-05T13:00:00Z');
INSERT INTO posts (run_key, content_type, template_id, trading_date, slug, published_at)
VALUES ('premarket-2026-01-05', 'premarket', 'premarket-v1', '2026-01-05', 'premarket-2026-01-05', '2026-01-05T13:25:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
SELECT id, 'en', 'published', 'Premarket Brief', 'Summary', 'Body', '2026-01-05T13:25:00Z' FROM posts WHERE slug = 'premarket-2026-01-05';
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
SELECT id, 'zh', 'published', '盘前简报', '摘要', '正文', '2026-01-05T13:25:00Z' FROM posts WHERE slug = 'premarket-2026-01-05';
SELECT p.slug, r.status AS run_status, t.lang, t.title
FROM posts p JOIN runs r ON r.run_key = p.run_key JOIN post_translations t ON t.post_id = p.id
ORDER BY t.lang;

-- 0006_covers.sql
ALTER TABLE posts ADD COLUMN cover_key TEXT;
ALTER TABLE post_translations ADD COLUMN cover_line1 TEXT;
ALTER TABLE post_translations ADD COLUMN cover_line2 TEXT;

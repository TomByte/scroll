-- The latest compacted Loro snapshot of each page, covering every update up to snapshot_seq.
ALTER TABLE pages
    ADD COLUMN snapshot_seq BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN snapshot     BYTEA;

-- An update folded into the snapshot keeps its row without bytes, so a late resend is still recognised.
ALTER TABLE page_updates ALTER COLUMN bytes DROP NOT NULL;

-- Record sync. Records are stored as the postcard encoding of the scroll-core types, with the fields that
-- validation looks up copied into columns.

-- The workspace's record sequence: the seq of its last committed transaction. Locking this row serialises commits.
ALTER TABLE workspaces ADD COLUMN last_record_seq BIGINT NOT NULL DEFAULT 0;

-- A page's record lives on the same row as its content, so a page id is bound to one workspace whichever
-- arrives first. `record` is NULL while the page has content but no record, or after the record is deleted.
ALTER TABLE pages
    ADD COLUMN record    BYTEA,
    ADD COLUMN parent_id UUID,
    ADD COLUMN row_of    UUID;
CREATE INDEX pages_parent_id ON pages (parent_id) WHERE record IS NOT NULL;
CREATE INDEX pages_row_of ON pages (row_of) WHERE record IS NOT NULL;

CREATE TABLE databases (
    id           UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    host_page    UUID NOT NULL,
    record       BYTEA NOT NULL
);
CREATE INDEX databases_workspace_id ON databases (workspace_id);
CREATE INDEX databases_host_page ON databases (host_page);

CREATE TABLE views (
    id           UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    database_id  UUID NOT NULL,
    record       BYTEA NOT NULL
);
CREATE INDEX views_workspace_id ON views (workspace_id);
CREATE INDEX views_database_id ON views (database_id);

CREATE TABLE property_values (
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    page_id      UUID NOT NULL,
    property_id  UUID NOT NULL,
    value        BYTEA NOT NULL,
    PRIMARY KEY (page_id, property_id)
);
CREATE INDEX property_values_workspace_id ON property_values (workspace_id);

-- Every committed transaction, for catch-up and for recognising resends. Never truncated.
CREATE TABLE record_transactions (
    workspace_id   UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    seq            BIGINT NOT NULL,
    transaction_id UUID NOT NULL,
    author_id      UUID NOT NULL REFERENCES users (id),
    transaction    BYTEA NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, seq),
    UNIQUE (workspace_id, transaction_id)
);

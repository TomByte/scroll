CREATE TABLE users (
    id            UUID PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Only a SHA-256 of each token is kept, so a database leak does not hand out live sessions.
CREATE TABLE sessions (
    token_hash BYTEA PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX sessions_user_id ON sessions (user_id);

CREATE TABLE workspaces (
    id         UUID PRIMARY KEY,
    name       TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE memberships (
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role         TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);
CREATE INDEX memberships_user_id ON memberships (user_id);

-- An invite without a workspace grants sign-up only; that is how the first user gets in.
CREATE TABLE invites (
    code_hash    BYTEA PRIMARY KEY,
    workspace_id UUID REFERENCES workspaces (id) ON DELETE CASCADE,
    role         TEXT CHECK (role IN ('owner', 'editor', 'viewer')),
    created_by   UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL,
    accepted_by  UUID REFERENCES users (id) ON DELETE SET NULL,
    accepted_at  TIMESTAMPTZ,
    CHECK ((workspace_id IS NULL) = (role IS NULL))
);

-- last_seq is the per-page counter; locking this row serialises pushes to one page.
CREATE TABLE pages (
    id           UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    last_seq     BIGINT NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX pages_workspace_id ON pages (workspace_id);

CREATE TABLE page_updates (
    page_id    UUID NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    seq        BIGINT NOT NULL,
    update_id  UUID NOT NULL,
    author_id  UUID NOT NULL REFERENCES users (id),
    bytes      BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (page_id, seq),
    UNIQUE (page_id, update_id)
);

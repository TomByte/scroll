-- An account may have a password, a Google identity, or both; a password is cleared when Google proves the email
-- belongs to someone else than whoever chose the password (see accounts::sign_in_with_google).
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;

-- An identity at a sign-in provider, by the provider's own stable id for the person (Google's `sub`): an email
-- can change hands, the subject cannot.
CREATE TABLE identities (
    provider   TEXT NOT NULL,
    subject    TEXT NOT NULL,
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email      TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject),
    UNIQUE (user_id, provider)
);

-- A browser sign-in between leaving for the provider and coming back. Secrets are stored hashed.
CREATE TABLE browser_sign_ins (
    state_hash    BYTEA PRIMARY KEY,
    nonce_hash    BYTEA NOT NULL,
    challenge     TEXT NOT NULL,
    redirect_port INTEGER NOT NULL,
    invite_hash   BYTEA,
    expires_at    TIMESTAMPTZ NOT NULL
);

-- The one-time code the app exchanges for a session, bound to the app's PKCE challenge.
CREATE TABLE sign_in_codes (
    code_hash  BYTEA PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    challenge  TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);

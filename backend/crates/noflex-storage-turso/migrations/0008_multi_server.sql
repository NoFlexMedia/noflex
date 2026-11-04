-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Sharing a server by invite, and the other servers a user has linked.

CREATE TABLE IF NOT EXISTS invites (
    id               BLOB PRIMARY KEY,
    -- SHA-256 hex; the plaintext only ever exists in the link.
    token_hash       TEXT NOT NULL UNIQUE,
    created_by       BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    library_ids_json TEXT NOT NULL,
    max_uses         INTEGER NOT NULL,
    uses             INTEGER NOT NULL DEFAULT 0,
    expires_at       TEXT NOT NULL,
    revoked_at       TEXT,
    created_at       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS linked_servers (
    id           BLOB PRIMARY KEY,
    user_id      BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    origin       TEXT NOT NULL,
    name         TEXT NOT NULL,
    -- This user's key on that server, returned to their clients after sign-in
    -- so linked servers follow them between devices. Held the way
    -- user_plugin_secrets holds other services' credentials.
    api_key      TEXT NOT NULL,
    sync_enabled INTEGER NOT NULL DEFAULT 1,
    synced_until TEXT,
    created_at   TEXT NOT NULL,
    UNIQUE (user_id, origin)
);

-- The web CSP is generated from every linked origin.
CREATE INDEX IF NOT EXISTS linked_servers_origin ON linked_servers(origin);

-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Vectors from different models can't be compared, so the index records the model
-- it belongs to, and notices when that changes. The endpoint is what an
-- administrator configured instead of the local embedder, when they did.

CREATE TABLE IF NOT EXISTS embedding_index (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    namespace    TEXT NOT NULL,
    model_id     TEXT NOT NULL,
    dimensions   INTEGER NOT NULL,
    activated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS embedding_endpoint (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    url                TEXT NOT NULL,
    api_key            TEXT,
    model              TEXT NOT NULL,
    dimensions         INTEGER NOT NULL,
    request_dimensions INTEGER,
    query_prefix       TEXT NOT NULL,
    document_prefix    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS embeddings_namespace ON embeddings(namespace);

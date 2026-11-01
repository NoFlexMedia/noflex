-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Play history: one row per viewing, assembled from progress
-- heartbeats. `root_work_id` (series / artist / movie) and
-- `library_id` are denormalised at write time so per-library and
-- top-played rollups need no parent-chain walk.

CREATE TABLE IF NOT EXISTS play_sessions (
    id           BLOB PRIMARY KEY,
    user_id      BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id      BLOB NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    root_work_id BLOB NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    library_id   BLOB NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    started_at   TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    watched_ms   INTEGER NOT NULL DEFAULT 0,
    position_ms  INTEGER NOT NULL DEFAULT 0,
    playing      INTEGER NOT NULL DEFAULT 0,
    ended        INTEGER NOT NULL DEFAULT 0,
    device       TEXT
);

CREATE INDEX IF NOT EXISTS play_sessions_started
    ON play_sessions(started_at DESC, id DESC);

-- `open_play_session`: a user's latest live viewing of one work.
CREATE INDEX IF NOT EXISTS play_sessions_user_work
    ON play_sessions(user_id, work_id, last_seen_at);

CREATE INDEX IF NOT EXISTS play_sessions_user_started
    ON play_sessions(user_id, started_at);

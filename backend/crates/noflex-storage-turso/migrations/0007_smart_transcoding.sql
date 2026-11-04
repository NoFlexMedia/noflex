-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Registered client devices, and the optimized versions prepared for them.
-- A version is derived from a media file and is never one itself: scans
-- prune media_files, and clients list those rows as real versions.

CREATE TABLE IF NOT EXISTS devices (
    -- Chosen by the client, so it survives signing out.
    id                    BLOB PRIMARY KEY,
    user_id               BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                  TEXT NOT NULL,
    capabilities_json     TEXT NOT NULL,
    smart_transcode       INTEGER NOT NULL DEFAULT 0,
    max_bitrate_bps       INTEGER,
    allowed_video_codecs_json TEXT NOT NULL DEFAULT '[]',
    smart_downloads       INTEGER NOT NULL DEFAULT 0,
    download_budget_bytes INTEGER,
    downloads_wifi_only   INTEGER NOT NULL DEFAULT 1,
    created_at            TEXT NOT NULL,
    last_seen_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS devices_user ON devices(user_id);

CREATE TABLE IF NOT EXISTS optimized_versions (
    id            BLOB PRIMARY KEY,
    media_file_id BLOB NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    -- What the version contains; the same plan is never prepared twice.
    variant_key   TEXT NOT NULL,
    status        TEXT NOT NULL,           -- "queued" | "running" | "ready" | "failed"
    -- Lower runs sooner.
    priority      INTEGER NOT NULL,
    plan_json     TEXT NOT NULL,
    -- The source this was made from; a version of an older file is stale.
    source_size   INTEGER NOT NULL,
    source_mtime  TEXT NOT NULL,
    progress      REAL NOT NULL DEFAULT 0.0,
    attempts      INTEGER NOT NULL DEFAULT 0,
    path          TEXT,
    size_bytes    INTEGER,
    index_json    TEXT,
    probe_json    TEXT,
    error         TEXT,
    created_at    TEXT NOT NULL,
    finished_at   TEXT,
    last_used_at  TEXT,
    UNIQUE (media_file_id, variant_key)
);

CREATE INDEX IF NOT EXISTS optimized_versions_claim ON optimized_versions(status, priority, created_at);

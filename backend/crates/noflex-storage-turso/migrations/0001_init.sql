-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Full schema for the Turso backend, squashed before the first release.
-- Going forward, schema changes land as new numbered files after this
-- one; a file that has shipped is never edited.

CREATE TABLE IF NOT EXISTS schema_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- `config_*` fields are JSON blobs / simple scalars so per-field
-- evolution doesn't require a migration per field.
CREATE TABLE IF NOT EXISTS libraries (
    id                     BLOB PRIMARY KEY,
    name                   TEXT NOT NULL,
    kind                   TEXT NOT NULL,
    paths_json             TEXT NOT NULL,
    config_languages_json  TEXT NOT NULL DEFAULT '[]',
    config_providers_json  TEXT NOT NULL DEFAULT '[]',
    config_excluded_json   TEXT NOT NULL DEFAULT '[]',
    config_watch_enabled   INTEGER NOT NULL DEFAULT 1,
    config_scan_schedule   TEXT NOT NULL DEFAULT 'DISABLED',
    -- Scanner layout version. When the scanner changes how it derives work
    -- identity from paths, a library recorded under an older version has
    -- every file re-planned once. 0 = never recorded.
    scan_version           INTEGER NOT NULL DEFAULT 0,
    -- Choice between provider and local artwork; provider-first by default.
    config_artwork_preference TEXT NOT NULL DEFAULT 'PREFER_PROVIDER',
    -- Override of anime classification for content-type browsing; by
    -- default a library classifies per work.
    config_content_category TEXT NOT NULL DEFAULT 'AUTO'
);

-- A Work whose `index_end` is non-NULL spans an inclusive index range
-- (`index_number` .. `index_end`). The canonical case is a multi-episode
-- file like `s01e01-e02` ⇒ index_number=1, index_end=2. Single-indexed
-- Works leave it NULL.
--
-- `metadata_status` is the per-Work confidence flag on the canonical
-- metadata columns (`title`, `year`, `overview`, …). Scanner-created
-- rows live at `unconfirmed`; metadata providers escalate to
-- `confirmed`; users escalate to `user_edited`. Frontends use this
-- to badge unverified entries and decide whether refreshes are safe.
CREATE TABLE IF NOT EXISTS works (
    id                  BLOB PRIMARY KEY,
    kind                TEXT NOT NULL,
    parent_id           BLOB,
    library_id          BLOB NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    title               TEXT NOT NULL,
    sort_title          TEXT NOT NULL,
    year                INTEGER,
    release_date        TEXT,
    overview            TEXT,
    external_ids_json   TEXT NOT NULL DEFAULT '[]',
    index_number        INTEGER,
    index_end           INTEGER,
    parent_index        INTEGER,
    metadata_status     TEXT NOT NULL DEFAULT 'unconfirmed',
    added_at            TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    tagline             TEXT,
    original_title      TEXT,
    community_rating    REAL,
    -- JSON array of snake_case `WorkField` names the user locked against
    -- provider refreshes; empty means nothing is locked.
    locked_fields_json  TEXT NOT NULL DEFAULT '[]',
    -- The plugin whose fetch last supplied this work's fields.
    metadata_provider   TEXT,
    -- The top of the work's parent chain, itself for a top-level work: what
    -- a file or a play counts towards, without walking up to it.
    root_id             BLOB
);

CREATE INDEX IF NOT EXISTS works_library_kind     ON works(library_id, kind);
CREATE INDEX IF NOT EXISTS works_parent_idx       ON works(parent_id, parent_index, index_number);
CREATE INDEX IF NOT EXISTS works_root ON works(root_id);
-- These let a grid sorted by them read in order and stop once a page is full.
CREATE INDEX IF NOT EXISTS works_added ON works(added_at, id);
CREATE INDEX IF NOT EXISTS works_year ON works(year, id);
CREATE INDEX IF NOT EXISTS works_released ON works(release_date, id);
CREATE INDEX IF NOT EXISTS works_rating ON works(community_rating, id);

CREATE TABLE IF NOT EXISTS work_external_ids (
    kind    TEXT NOT NULL,
    value   TEXT NOT NULL,
    work_id BLOB NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    PRIMARY KEY (kind, value)
);

CREATE INDEX IF NOT EXISTS work_external_ids_work ON work_external_ids(work_id);

-- An edition is a cut, and can hold several files with different pictures and
-- sound, so facts about a file can't live on it. They're in `media_streams`.
CREATE TABLE IF NOT EXISTS editions (
    id                         BLOB PRIMARY KEY,
    work_id                    BLOB NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    name                       TEXT NOT NULL,
    runtime_ms                 INTEGER,
    intro_start_ms             INTEGER,
    intro_end_ms               INTEGER,
    credits_start_ms           INTEGER,
    chapters_json              TEXT NOT NULL DEFAULT '[]'
);

-- One edition per name within a work; also serves work_id lookups.
CREATE UNIQUE INDEX IF NOT EXISTS editions_work_name ON editions(work_id, name);
CREATE INDEX IF NOT EXISTS editions_name ON editions(name);

CREATE TABLE IF NOT EXISTS media_files (
    id          BLOB PRIMARY KEY,
    edition_id  BLOB NOT NULL REFERENCES editions(id) ON DELETE CASCADE,
    path        TEXT NOT NULL UNIQUE,
    part_index  INTEGER NOT NULL DEFAULT 0,
    size_bytes  INTEGER NOT NULL,
    mtime       TEXT NOT NULL,
    container   TEXT,
    probe_json  TEXT NOT NULL DEFAULT '{}',
    subtitle_tracks_json TEXT NOT NULL DEFAULT '[]'
);

CREATE INDEX IF NOT EXISTS media_files_edition ON media_files(edition_id);

CREATE TABLE IF NOT EXISTS scan_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    library_id  BLOB NOT NULL,
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    added       INTEGER NOT NULL,
    updated     INTEGER NOT NULL,
    removed     INTEGER NOT NULL,
    errored     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS scan_events_library_started ON scan_events(library_id, started_at);

-- `password_hash` is opaque text (Argon2id by default, legacy
-- Jellyfin-SHA1 at the auth-provider layer); OIDC/LDAP-only users
-- write the empty string. `is_admin` stays for backward compat; the
-- `role` column is canonical for the role middleware.
-- `username` is what they sign in with, `display_name` what they're called.
-- `pin_hash` is the Argon2id hash of the PIN that unlocks their sessions on a
-- device they share; NULL without one. It never signs anyone in on its own.
CREATE TABLE IF NOT EXISTS users (
    id            BLOB PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    display_name  TEXT,
    password_hash TEXT NOT NULL,
    pin_hash      TEXT,
    is_admin      INTEGER NOT NULL,
    role          TEXT NOT NULL DEFAULT 'user',
    created_at    TEXT NOT NULL
);

-- Apart from `users`, which every request reads and has no use for the bytes.
CREATE TABLE IF NOT EXISTS user_avatars (
    user_id    BLOB PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    image      BLOB NOT NULL,
    updated_at TEXT NOT NULL
);

-- Sessions FK CASCADE on user delete, so revoking a user also revokes
-- their active sessions.
CREATE TABLE IF NOT EXISTS sessions (
    id                 BLOB PRIMARY KEY,
    user_id            BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client             TEXT,
    device             TEXT,
    device_id          TEXT,
    app_version        TEXT,
    remote_addr        TEXT,
    created_at         TEXT NOT NULL,
    last_seen_at       TEXT NOT NULL,
    -- SHA-256 hex of the refresh token; the plaintext never touches the
    -- DB. Nullable so legacy rows and non-token sessions stay valid;
    -- SQLite treats NULLs as distinct under UNIQUE.
    refresh_token_hash TEXT,
    expires_at         TEXT NOT NULL,
    -- Set while the session waits for its user's PIN: nothing it issued
    -- works until then. `pin_failures` counts wrong ones since it last opened.
    locked_at          TEXT,
    pin_failures       INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS sessions_user ON sessions(user_id);
CREATE INDEX IF NOT EXISTS sessions_last_seen ON sessions(last_seen_at);
CREATE UNIQUE INDEX IF NOT EXISTS sessions_refresh_token ON sessions(refresh_token_hash);

-- Every refresh-token hash a session has rotated away from, kept for the
-- session's lifetime so replay of any superseded token is detectable
-- (RFC 9700 §4.14.2), and deleted with the session row. `replaced_by`
-- is the hash that rotation installed; it equals the session's current
-- hash exactly when this is the immediately-previous token.
CREATE TABLE IF NOT EXISTS retired_refresh_tokens (
    token_hash  TEXT PRIMARY KEY,
    session_id  BLOB NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    replaced_by TEXT NOT NULL,
    retired_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS retired_refresh_tokens_session
    ON retired_refresh_tokens(session_id);

CREATE TABLE IF NOT EXISTS user_data (
    user_id      BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_kind  TEXT NOT NULL,
    target_id    BLOB NOT NULL,
    played       INTEGER NOT NULL DEFAULT 0,
    position_ms  INTEGER,
    play_count   INTEGER NOT NULL DEFAULT 0,
    rating       REAL,
    favorite     INTEGER NOT NULL DEFAULT 0,
    last_played  TEXT,
    PRIMARY KEY (user_id, target_kind, target_id)
);

CREATE INDEX IF NOT EXISTS user_data_user_played
    ON user_data(user_id, played, last_played);

CREATE INDEX IF NOT EXISTS user_data_user_favorite
    ON user_data(user_id, favorite);

CREATE INDEX IF NOT EXISTS user_data_target
    ON user_data(target_kind, target_id);

-- `list_play_history` keyset order.
CREATE INDEX IF NOT EXISTS user_data_history
    ON user_data(user_id, last_played DESC, target_kind DESC, target_id DESC);

CREATE TABLE IF NOT EXISTS tags (
    id         BLOB PRIMARY KEY,
    namespace  TEXT NOT NULL,
    slug       TEXT NOT NULL,
    display    TEXT NOT NULL,
    parent_id  BLOB REFERENCES tags(id) ON DELETE SET NULL,
    metadata   TEXT NOT NULL DEFAULT '{}',
    UNIQUE (namespace, slug)
);

CREATE INDEX IF NOT EXISTS tags_namespace ON tags(namespace);

CREATE TABLE IF NOT EXISTS tag_assignments (
    tag_id      BLOB NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL,
    target_id   BLOB NOT NULL,
    weight      REAL NOT NULL DEFAULT 1.0,
    source      TEXT NOT NULL,
    -- What's true of a tag on one target rather than of the tag: a person's
    -- role and character belong to the work, not to the person.
    metadata    TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (tag_id, target_kind, target_id)
);

CREATE INDEX IF NOT EXISTS tag_assignments_target
    ON tag_assignments(target_kind, target_id);

-- `source` is part of the primary key, so a work can hold a local and a
-- provider poster at once and readers choose between them by preference. The
-- primary key also serves lookups by target.
CREATE TABLE IF NOT EXISTS images (
    target_kind TEXT NOT NULL,
    target_id   BLOB NOT NULL,
    kind        TEXT NOT NULL,
    source      TEXT NOT NULL CHECK (source IN ('local', 'provider', 'generated')),
    path        TEXT NOT NULL,
    width       INTEGER,
    height      INTEGER,
    language    TEXT,
    -- The plugin behind a `provider` row.
    provider    TEXT,
    PRIMARY KEY (target_kind, target_id, kind, source)
);

-- How each metadata plugin asks to be credited. Kept past the plugin's removal,
-- because the titles it described still show what it supplied.
CREATE TABLE IF NOT EXISTS provider_attributions (
    provider    TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    text        TEXT NOT NULL,
    url         TEXT,
    logo        BLOB,
    logo_mime   TEXT,
    logo_digest TEXT
);

-- One rating per work and source, from whichever plugin last reported that
-- source. `normalized` is `value` out of 10; `works.community_rating` is the
-- `normalized` of the work's first row in `rating_sources` order.
CREATE TABLE IF NOT EXISTS work_ratings (
    work_id    BLOB NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    source     TEXT NOT NULL,
    provider   TEXT NOT NULL,
    value      REAL NOT NULL,
    scale      REAL NOT NULL,
    normalized REAL NOT NULL,
    votes      INTEGER,
    PRIMARY KEY (work_id, source)
);

-- The plugins that have rated a work, whether or not they had a rating, so one
-- loaded later is asked about the works it hasn't seen.
CREATE TABLE IF NOT EXISTS work_rating_checks (
    work_id  BLOB NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    PRIMARY KEY (work_id, provider)
);

-- The sources `works.community_rating` is taken from, `rank` 0 first. The
-- server writes its configured list at every start; these are its defaults
-- (`DEFAULT_RATING_SOURCES`).
CREATE TABLE IF NOT EXISTS rating_sources (
    rank   INTEGER PRIMARY KEY,
    source TEXT NOT NULL UNIQUE
);

INSERT INTO rating_sources (rank, source) VALUES
    (0, 'imdb'), (1, 'tmdb'), (2, 'tvdb'), (3, 'musicbrainz');

-- Lexical search ranks with BM25 over Tantivy indexes. Names index every
-- 1- to 3-gram so part of a word finds them as someone types; a synopsis
-- indexes whole words, as grams of prose would match nearly anything.
-- Hybrid fusing lives in `noflex-storage/src/search.rs` (RRF).
CREATE INDEX IF NOT EXISTS works_title_fts ON works USING fts (title, sort_title)
    WITH (tokenizer = 'ngram', min_gram = 1, max_gram = 3);
CREATE INDEX IF NOT EXISTS works_overview_fts ON works USING fts (overview);
CREATE INDEX IF NOT EXISTS tags_display_fts ON tags USING fts (display)
    WITH (tokenizer = 'ngram', min_gram = 1, max_gram = 3);

-- Vectors are raw little-endian f32, which is the engine's `vector32`
-- format, so `vector_distance_cos` reads them as stored.
CREATE TABLE IF NOT EXISTS embeddings (
    target_kind TEXT NOT NULL,
    target_id   BLOB NOT NULL,
    namespace   TEXT NOT NULL,
    model_id    TEXT NOT NULL,
    dimensions  INTEGER NOT NULL,
    vector_blob BLOB NOT NULL,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (target_kind, target_id, namespace, model_id)
);

CREATE INDEX IF NOT EXISTS embeddings_target
    ON embeddings(target_kind, target_id);
CREATE INDEX IF NOT EXISTS embeddings_namespace ON embeddings(namespace);

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

-- Admins see every library; non-admins only see rows in
-- user_library_access. Guests default to no-access.
CREATE TABLE IF NOT EXISTS user_library_access (
    user_id    BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    library_id BLOB NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    granted_at TEXT NOT NULL,
    PRIMARY KEY (user_id, library_id)
);

CREATE INDEX IF NOT EXISTS user_library_access_user
    ON user_library_access(user_id);

-- Per-(plugin, hook, work_kind) priority overrides. The host reads the
-- active set on startup to build the PriorityTable, and the GraphQL
-- `setPluginOverride` mutation writes here.
CREATE TABLE IF NOT EXISTS plugin_overrides (
    plugin_id   TEXT NOT NULL,
    hook        TEXT NOT NULL,
    work_kind   TEXT NOT NULL,
    priority    INTEGER NOT NULL,
    PRIMARY KEY (plugin_id, hook, work_kind)
);

-- Plugin settings an administrator set from the admin UI, over the
-- server's config.toml stanza; clearing a row falls back to it. Secrets
-- sit in a table of their own, which only the host's `host-secrets.read`
-- reads values from. No FK on plugin_id: plugins are not storage rows.
CREATE TABLE IF NOT EXISTS plugin_settings (
    plugin_id   TEXT NOT NULL,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (plugin_id, key)
);

CREATE TABLE IF NOT EXISTS plugin_secret_settings (
    plugin_id   TEXT NOT NULL,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (plugin_id, key)
);

-- Append-only activity log. Three flavors of event:
--   - admin actions (user create/delete, library create/delete, ...)
--   - scan events (already persisted in scan_events; logged here
--     too for a unified "what just happened?" view)
--   - play events (resume position changes, played/unplayed,
--     scrobble notifications)
CREATE TABLE IF NOT EXISTS activity_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT    NOT NULL,
    actor       TEXT,
    subject     TEXT,
    detail      TEXT,
    occurred_at TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS activity_kind_time
    ON activity_log(kind, occurred_at DESC);

CREATE INDEX IF NOT EXISTS activity_time
    ON activity_log(occurred_at DESC);

-- A user kicks off an export of an Edition, the worker consumes the
-- queue, and the user can download the result with HTTP range support.
-- Mirrors Plex's /SyncJobs endpoint.
CREATE TABLE IF NOT EXISTS export_jobs (
    id           BLOB PRIMARY KEY,
    user_id      BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    edition_id   BLOB NOT NULL REFERENCES editions(id) ON DELETE CASCADE,
    format       TEXT NOT NULL,           -- "mp4" | "mkv"
    status       TEXT NOT NULL,           -- "queued" | "running" | "completed" | "failed" | "cancelled"
    progress     REAL NOT NULL DEFAULT 0.0,
    output_path  TEXT,
    error        TEXT,
    created_at   TEXT NOT NULL,
    started_at   TEXT,
    finished_at  TEXT,
    -- Lease held by the worker running the job; an expired lease lets
    -- another worker reclaim it.
    worker_id        TEXT,
    lease_token      TEXT,
    lease_expires_at TEXT,
    attempt_count    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS export_jobs_user ON export_jobs(user_id, created_at);
CREATE INDEX IF NOT EXISTS export_jobs_claim ON export_jobs(status, lease_expires_at, created_at);

CREATE TABLE IF NOT EXISTS user_preferences (
    user_id      BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key          TEXT NOT NULL,
    value        TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (user_id, key)
);

-- Collections are admin-curated and library-scoped; playlists are
-- user-owned. Item lists live in side tables with an explicit `position`
-- (unique per list); all membership rows cascade away with their parent.
CREATE TABLE IF NOT EXISTS collections (
    id          BLOB PRIMARY KEY,
    library_id  BLOB NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    sort_title  TEXT NOT NULL,
    overview    TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS collections_library_sort
    ON collections(library_id, sort_title, id);

CREATE TABLE IF NOT EXISTS collection_items (
    collection_id BLOB NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    work_id       BLOB NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    position      INTEGER NOT NULL,
    PRIMARY KEY (collection_id, work_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS collection_items_position
    ON collection_items(collection_id, position);
CREATE INDEX IF NOT EXISTS collection_items_page
    ON collection_items(collection_id, position, work_id);

CREATE TABLE IF NOT EXISTS playlists (
    id         BLOB PRIMARY KEY,
    owner_id   BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title      TEXT NOT NULL,
    overview   TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS playlists_owner_title
    ON playlists(owner_id, title, id);

CREATE TABLE IF NOT EXISTS playlist_items (
    playlist_id BLOB NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL,            -- "work" | "edition"
    target_id   BLOB NOT NULL,
    position    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, target_kind, target_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS playlist_items_position
    ON playlist_items(playlist_id, position);
CREATE INDEX IF NOT EXISTS playlist_items_page
    ON playlist_items(playlist_id, position, target_kind, target_id);
-- Reverse lookup when a work or edition is pruned.
CREATE INDEX IF NOT EXISTS playlist_items_target
    ON playlist_items(target_kind, target_id);

-- API keys and device-linking codes. Both FK CASCADE on user delete, so
-- revoking a user wipes every credential they ever minted. The key hash
-- and code are the lookup paths; plaintexts never touch the DB.
CREATE TABLE IF NOT EXISTS api_keys (
    id         BLOB PRIMARY KEY,
    user_id    BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    -- SHA-256 hex of the presented key; UNIQUE so authentication is a
    -- single index probe.
    key_hash   TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    -- NULL = active. Revocation keeps the row for audit; callers check
    -- revoked_at on a hash hit.
    revoked_at TEXT
);

CREATE INDEX IF NOT EXISTS api_keys_user ON api_keys(user_id);

-- `user_id` is NULL until a signed-in user authorizes the code; an
-- expired row is garbage the backend may delete lazily on lookup.
CREATE TABLE IF NOT EXISTS device_codes (
    id         BLOB PRIMARY KEY,
    code       TEXT NOT NULL UNIQUE,
    user_id    BLOB REFERENCES users(id) ON DELETE CASCADE,
    client     TEXT,
    device     TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

-- External sign-in identities, keyed by the verified (issuer, subject)
-- pair a plugin reports. One identity per (user, plugin).
CREATE TABLE IF NOT EXISTS external_identities (
    plugin_id    TEXT NOT NULL,
    issuer       TEXT NOT NULL,
    subject      TEXT NOT NULL,
    user_id      BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    username     TEXT,
    display_name TEXT,
    email        TEXT,
    linked_at    TEXT NOT NULL,
    PRIMARY KEY (plugin_id, issuer, subject),
    UNIQUE (user_id, plugin_id)
);

-- Per-user playback policy. An empty rating list allows every rating;
-- unrated works are governed by `allow_unrated` alone.
CREATE TABLE IF NOT EXISTS user_restrictions (
    user_id                   BLOB PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    allow_unrated             INTEGER NOT NULL,
    max_stream_bitrate_bps    INTEGER,
    max_concurrent_transcodes INTEGER
);

CREATE TABLE IF NOT EXISTS user_restriction_ratings (
    user_id BLOB NOT NULL REFERENCES user_restrictions(user_id) ON DELETE CASCADE,
    tag_id  BLOB NOT NULL REFERENCES tags(id) ON DELETE RESTRICT,
    PRIMARY KEY (user_id, tag_id)
);

-- Host-side bookkeeping for plugin background tasks. Plugins never see
-- these tables; the plugin-host scheduler is the only writer. No FKs —
-- rows belong to plugins, not users, and uninstall cleanup is an
-- explicit host call, not a cascade.

-- Progress cursor + health, one row per (plugin, task).
CREATE TABLE IF NOT EXISTS plugin_task_state (
    plugin_id            TEXT NOT NULL,
    task                 TEXT NOT NULL,
    cursor               TEXT,
    last_run_at          TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (plugin_id, task)
);

-- FIFO of dynamically enqueued payloads (host-tasks.enqueue). `id`
-- AUTOINCREMENT gives the dequeue order; one queue per (plugin, task).
CREATE TABLE IF NOT EXISTS plugin_task_queue (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_id   TEXT NOT NULL,
    task        TEXT NOT NULL,
    payload     TEXT,
    enqueued_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_plugin_task_queue_order
    ON plugin_task_queue (plugin_id, task, id);

-- Per-user plugin credentials (OAuth tokens, API keys) written by the
-- host after a plugin link flow. Plugins never see `Storage` — the host
-- reads these on a plugin's behalf. No FK on plugin_id: plugins are not
-- storage rows, and uninstall cleanup is an explicit host call.
CREATE TABLE IF NOT EXISTS user_plugin_secrets (
    user_id    BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    plugin_id  TEXT NOT NULL,
    key        TEXT NOT NULL,
    value      TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, plugin_id, key)
);

-- Scrubbing thumbnails: one asset per edition, sprite sheets beneath it.
-- `source_fingerprint` identifies the media it was rendered from (see the
-- field's contract in noflex-storage) so stale assets regenerate.
CREATE TABLE IF NOT EXISTS trickplay_assets (
    edition_id          BLOB PRIMARY KEY REFERENCES editions(id) ON DELETE CASCADE,
    vtt_path            TEXT NOT NULL,
    thumb_width         INTEGER NOT NULL,
    thumb_height        INTEGER NOT NULL,
    tile_columns        INTEGER NOT NULL,
    tile_rows           INTEGER NOT NULL,
    interval_ms         INTEGER NOT NULL,
    source_media_mtime  TEXT NOT NULL,
    source_fingerprint  TEXT NOT NULL,
    generated_at        TEXT NOT NULL,
    chapter_cues_json   TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS trickplay_sheets (
    edition_id  BLOB NOT NULL REFERENCES trickplay_assets(edition_id) ON DELETE CASCADE,
    sheet_index INTEGER NOT NULL,
    path        TEXT NOT NULL,
    tile_count  INTEGER NOT NULL,
    PRIMARY KEY (edition_id, sheet_index)
);

-- Synchronised playback rooms. `sequence` increments on every control
-- event so clients can discard stale updates.
CREATE TABLE IF NOT EXISTS watch_rooms (
    id            BLOB PRIMARY KEY,
    host_user_id  BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    media_file_id BLOB NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    state         TEXT NOT NULL CHECK (state IN ('waiting', 'playing', 'paused', 'ended')),
    position_ms   INTEGER NOT NULL CHECK (position_ms >= 0),
    sequence      INTEGER NOT NULL CHECK (sequence >= 0),
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS watch_rooms_host ON watch_rooms(host_user_id);

CREATE TABLE IF NOT EXISTS watch_room_members (
    room_id   BLOB NOT NULL REFERENCES watch_rooms(id) ON DELETE CASCADE,
    user_id   BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at TEXT NOT NULL,
    PRIMARY KEY (room_id, user_id)
);

CREATE INDEX IF NOT EXISTS watch_room_members_user ON watch_room_members(user_id);

-- Live TV: M3U tuner sources, their channels, and XMLTV guide data.
CREATE TABLE IF NOT EXISTS live_tv_sources (
    id              BLOB PRIMARY KEY NOT NULL,
    name            TEXT NOT NULL,
    enabled         INTEGER NOT NULL,
    playlist_url    TEXT NOT NULL,
    epg_url         TEXT,
    last_refresh_at TEXT,
    last_error      TEXT
);

CREATE TABLE IF NOT EXISTS live_tv_channels (
    id            BLOB PRIMARY KEY NOT NULL,
    source_id     BLOB NOT NULL REFERENCES live_tv_sources(id) ON DELETE CASCADE,
    external_id   TEXT NOT NULL,
    name          TEXT NOT NULL,
    number        TEXT,
    logo_url      TEXT,
    channel_group TEXT,
    enabled       INTEGER NOT NULL,
    stream_url    TEXT NOT NULL,
    UNIQUE (source_id, external_id)
);

CREATE INDEX IF NOT EXISTS live_tv_channels_source_enabled
    ON live_tv_channels(source_id, enabled, id);

-- `refresh_id` stamps the guide refresh that last wrote the row; rows
-- with a stale stamp were absent from the latest feed and are swept.
CREATE TABLE IF NOT EXISTS epg_programs (
    id          BLOB PRIMARY KEY NOT NULL,
    channel_id  BLOB NOT NULL REFERENCES live_tv_channels(id) ON DELETE CASCADE,
    external_id TEXT NOT NULL,
    title       TEXT NOT NULL,
    subtitle    TEXT,
    description TEXT,
    category    TEXT,
    start_at    TEXT NOT NULL,
    end_at      TEXT NOT NULL,
    refresh_id  BLOB,
    UNIQUE (channel_id, external_id, start_at)
);

CREATE INDEX IF NOT EXISTS epg_programs_guide ON epg_programs(channel_id, start_at, end_at);

-- DVR: a schedule fires once into a recording; recordings outlive their
-- schedule and channel so finished files stay listable.
CREATE TABLE IF NOT EXISTS recording_schedules (
    id                  BLOB PRIMARY KEY NOT NULL,
    owner_id            BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id          BLOB NOT NULL REFERENCES live_tv_channels(id) ON DELETE CASCADE,
    program_id          BLOB REFERENCES epg_programs(id) ON DELETE SET NULL,
    start_at            TEXT NOT NULL,
    end_at              TEXT NOT NULL,
    padding_before_secs INTEGER NOT NULL,
    padding_after_secs  INTEGER NOT NULL,
    status              TEXT NOT NULL CHECK (status IN ('scheduled', 'cancelled', 'triggered')),
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS recording_schedules_due ON recording_schedules(status, start_at);
CREATE INDEX IF NOT EXISTS recording_schedules_owner_id ON recording_schedules(owner_id, id);

CREATE TABLE IF NOT EXISTS recordings (
    id           BLOB PRIMARY KEY NOT NULL,
    schedule_id  BLOB UNIQUE REFERENCES recording_schedules(id) ON DELETE SET NULL,
    owner_id     BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id   BLOB REFERENCES live_tv_channels(id) ON DELETE SET NULL,
    status       TEXT NOT NULL CHECK (status IN ('scheduled', 'running', 'completed', 'failed', 'cancelled')),
    path         TEXT,
    error        TEXT,
    size_bytes   INTEGER,
    started_at   TEXT,
    completed_at TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS recordings_owner_id ON recordings(owner_id, id);
CREATE INDEX IF NOT EXISTS recordings_status ON recordings(status, updated_at);

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

-- Registered client devices, and the optimized versions prepared for them.
-- A version is derived from a media file and is never one itself: scans
-- prune media_files, and clients list those rows as real versions.

CREATE TABLE IF NOT EXISTS devices (
    -- Chosen by the client, so it survives signing out.
    id                    BLOB PRIMARY KEY,
    user_id               BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                  TEXT NOT NULL,
    -- The app it's running, which often says what a bare model or hostname doesn't.
    client                TEXT,
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
    -- Since when no plan has wanted this; each plan that does clears it.
    unwanted_since TEXT,
    UNIQUE (media_file_id, variant_key)
);

CREATE INDEX IF NOT EXISTS optimized_versions_claim ON optimized_versions(status, priority, created_at);

-- A subtitle a viewer fetched from a provider for a media file. The text is a
-- file in the server's subtitle cache, never in the media folder; it goes with
-- the media file, and the sweep removes it once unused for long enough.
CREATE TABLE IF NOT EXISTS downloaded_subtitles (
    id               BLOB PRIMARY KEY,
    media_file_id    BLOB NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    provider         TEXT NOT NULL,
    handle           TEXT NOT NULL,
    language         TEXT NOT NULL,
    format           TEXT NOT NULL,         -- SubtitleCodec::as_str
    release          TEXT,
    hearing_impaired INTEGER NOT NULL,
    forced           INTEGER NOT NULL,
    hash_match       INTEGER NOT NULL,
    file_name        TEXT NOT NULL,
    downloaded_by    BLOB REFERENCES users(id) ON DELETE SET NULL,
    created_at       TEXT NOT NULL,
    last_used_at     TEXT NOT NULL,
    UNIQUE (media_file_id, provider, handle)
);

CREATE INDEX IF NOT EXISTS downloaded_subtitles_last_used ON downloaded_subtitles(last_used_at);

-- A viewer marking a title they're going to watch: a hard signal to prepare it,
-- ahead of anything the planner guesses at, until they unmark it or watch it.
CREATE TABLE IF NOT EXISTS smart_transcode_marks (
    user_id             BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id             BLOB NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    -- The one device this is for. Empty, not null, for all of the viewer's
    -- devices with smart transcoding on: a null would escape the primary key.
    -- The planner drops a mark whose device has been forgotten.
    device_id           BLOB NOT NULL,
    created_at          TEXT NOT NULL,
    -- What the planner last made of it, null until it has looked.
    outcome             TEXT,            -- "planned" | "not_needed" | "too_large" | "no_devices"
    -- The files it stands for right now; a series moves on as it's watched.
    media_file_ids_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (user_id, work_id, device_id)
);

CREATE INDEX IF NOT EXISTS smart_transcode_marks_queue ON smart_transcode_marks(created_at);

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

-- What an administrator changed about smart transcoding from the web UI. A null
-- column falls back to the server's configuration.

CREATE TABLE IF NOT EXISTS smart_transcoding_settings (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    dir            TEXT,
    max_storage_gb INTEGER,
    max_version_gb INTEGER
);

-- Server-wide choices an administrator made that have no configuration key.

CREATE TABLE IF NOT EXISTS server_settings (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    downloads_allowed INTEGER NOT NULL DEFAULT 1
);

-- Transcoding choices an administrator made from the web UI. A null column falls
-- back to the server's configuration.

CREATE TABLE IF NOT EXISTS transcoding_settings (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    tonemap_curve TEXT
);

-- What a file holds, in columns that can be indexed: filters, and anything else
-- asking "which titles are 4K", shouldn't have to read every probe to find out.
-- `media_files.probe_json` stays the raw record these are derived from.
--
-- Class table inheritance: every stream has a kind, codec and language, and only
-- its kind's table holds what's specific to it. Filled when a file is written.

CREATE TABLE IF NOT EXISTS media_streams (
    id            BLOB PRIMARY KEY,
    media_file_id BLOB NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL CHECK (kind IN ('video', 'audio', 'subtitle')),
    position      INTEGER NOT NULL,
    codec         TEXT,
    -- Canonical (`canonical_language`), null when undetermined.
    language      TEXT
);

CREATE INDEX IF NOT EXISTS media_streams_file ON media_streams(media_file_id);
CREATE INDEX IF NOT EXISTS media_streams_codec ON media_streams(kind, codec);
CREATE INDEX IF NOT EXISTS media_streams_language ON media_streams(kind, language);

CREATE TABLE IF NOT EXISTS video_streams (
    stream_id     BLOB PRIMARY KEY REFERENCES media_streams(id) ON DELETE CASCADE,
    width         INTEGER NOT NULL,
    height        INTEGER NOT NULL,
    resolution    TEXT NOT NULL,
    dynamic_range TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS video_streams_resolution ON video_streams(resolution);
CREATE INDEX IF NOT EXISTS video_streams_dynamic_range ON video_streams(dynamic_range);

CREATE TABLE IF NOT EXISTS audio_streams (
    stream_id BLOB PRIMARY KEY REFERENCES media_streams(id) ON DELETE CASCADE,
    channels  INTEGER NOT NULL,
    layout    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS audio_streams_layout ON audio_streams(layout);

CREATE TABLE IF NOT EXISTS subtitle_streams (
    stream_id BLOB PRIMARY KEY REFERENCES media_streams(id) ON DELETE CASCADE,
    -- A sidecar file beside the video, rather than muxed into it.
    external  INTEGER NOT NULL,
    forced    INTEGER NOT NULL
);

-- Deleting an account. The user row and everything cascading from it go at
-- once; what can't cascade waits here to be cleared in the background: rows
-- naming them in `activity_log`, which has no foreign key, and files on disk.
-- A row left by a restart is finished at the next start. No foreign key, as
-- the user it names is already gone.
CREATE TABLE IF NOT EXISTS account_purges (
    user_id      BLOB PRIMARY KEY,
    -- Their export copies and recordings. A copy someone else's job shares stays.
    files_json   TEXT NOT NULL,
    requested_at TEXT NOT NULL
);

-- The last administrator deleted their account, which erases the server back
-- to first-run setup. Every other table is emptied at the next start, and this
-- row goes last, so an erasure cut short is taken up again.
CREATE TABLE IF NOT EXISTS server_reset (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    -- Folders the configuration doesn't name that hold optimized versions.
    folders_json TEXT NOT NULL,
    requested_at TEXT NOT NULL
);

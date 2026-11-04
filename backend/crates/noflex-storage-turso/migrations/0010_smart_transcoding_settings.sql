-- SPDX-License-Identifier: AGPL-3.0-only
--
-- What an administrator changed about smart transcoding from the web UI. A null
-- column falls back to the server's configuration.

CREATE TABLE IF NOT EXISTS smart_transcoding_settings (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    dir            TEXT,
    max_storage_gb INTEGER,
    max_version_gb INTEGER
);

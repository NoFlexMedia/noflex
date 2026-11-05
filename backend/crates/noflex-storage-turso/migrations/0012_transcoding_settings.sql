-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Transcoding choices an administrator made from the web UI. A null column falls
-- back to the server's configuration.

CREATE TABLE IF NOT EXISTS transcoding_settings (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    tonemap_curve TEXT
);

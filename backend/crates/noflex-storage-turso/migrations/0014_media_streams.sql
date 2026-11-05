-- SPDX-License-Identifier: AGPL-3.0-only
--
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

CREATE INDEX IF NOT EXISTS editions_name ON editions(name);

-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Images gain a `source` (local / provider / generated) and it joins the
-- primary key, so a work can hold a local and a provider poster at once
-- and readers choose between them by preference.
--
-- Until now only the scanner (local files) and the episode-thumbnail
-- sweep (`<image cache>/episode-thumbs/`) wrote rows, so the backfill
-- classifies by that path. The old `images_target` index is not
-- recreated: the new primary key leads with the same columns.

CREATE TABLE images_v2 (
    target_kind TEXT NOT NULL,
    target_id   BLOB NOT NULL,
    kind        TEXT NOT NULL,
    source      TEXT NOT NULL CHECK (source IN ('local', 'provider', 'generated')),
    path        TEXT NOT NULL,
    width       INTEGER,
    height      INTEGER,
    language    TEXT,
    PRIMARY KEY (target_kind, target_id, kind, source)
);

INSERT INTO images_v2 (target_kind, target_id, kind, source, path, width, height, language)
SELECT target_kind, target_id, kind,
       CASE WHEN path LIKE '%/episode-thumbs/%' THEN 'generated' ELSE 'local' END,
       path, width, height, language
FROM images;

DROP INDEX IF EXISTS images_target;
DROP TABLE images;
ALTER TABLE images_v2 RENAME TO images;

-- SPDX-License-Identifier: AGPL-3.0-only
--
-- `root_id` is the top of a work's parent chain, itself for a top-level work: what
-- a file or a play counts towards, without walking up to it. The indexes let a grid
-- sorted by these read in order and stop once a page is full.

ALTER TABLE works ADD COLUMN root_id BLOB;

CREATE INDEX IF NOT EXISTS works_root ON works(root_id);
CREATE INDEX IF NOT EXISTS works_added ON works(added_at, id);
CREATE INDEX IF NOT EXISTS works_year ON works(year, id);
CREATE INDEX IF NOT EXISTS works_released ON works(release_date, id);

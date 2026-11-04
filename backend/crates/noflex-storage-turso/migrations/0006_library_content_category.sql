-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Per-library override of anime classification for content-type browsing.
-- Existing libraries classify per work.

ALTER TABLE libraries ADD COLUMN config_content_category TEXT NOT NULL DEFAULT 'AUTO';

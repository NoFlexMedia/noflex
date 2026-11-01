-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Scanner layout version per library. When the scanner changes how it
-- derives work identity from paths, a library recorded under an older
-- version has every file re-planned once. 0 = never recorded.

ALTER TABLE libraries ADD COLUMN scan_version INTEGER NOT NULL DEFAULT 0;

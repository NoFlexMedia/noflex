-- SPDX-License-Identifier: AGPL-3.0-only
--
-- Per-library choice between provider and local artwork. Existing
-- libraries take the default, provider-first.

ALTER TABLE libraries ADD COLUMN config_artwork_preference TEXT NOT NULL DEFAULT 'PREFER_PROVIDER';
